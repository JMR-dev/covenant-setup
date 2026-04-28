[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$ResultPath,
    [int]$TimeoutSeconds = 600,
    [string]$TracePath = $(Join-Path (Split-Path -Parent $ResultPath) "trace"),
    [string]$OperationName = "installer",
    [string]$InstallerArgumentsBase64 = "",
    [string[]]$InstallerArguments = @("--headed", "--automation")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Write-TraceEvent {
    param(
        [Parameter(Mandatory)][string]$Phase,
        [object]$Detail = $null
    )

    try {
        New-Item -ItemType Directory -Force -Path $TracePath | Out-Null
        $event = [ordered]@{
            time   = (Get-Date).ToUniversalTime().ToString("o")
            pid    = $PID
            script = Split-Path -Leaf $PSCommandPath
            phase  = $Phase
            detail = $Detail
        }
        $event | ConvertTo-Json -Depth 12 -Compress | Add-Content -LiteralPath (Join-Path $TracePath "guest-events.jsonl") -Encoding UTF8
    }
    catch {
        Write-Warning "Failed to write trace event '$Phase': $($_.Exception.Message)"
    }
}

function Write-DiagnosticFile {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$Capture
    )

    $path = Join-Path $TracePath $Name
    try {
        $value = & $Capture
        $value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding UTF8
    }
    catch {
        [ordered]@{
            error = $_.Exception.Message
            type  = $_.Exception.GetType().FullName
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $path -Encoding UTF8
    }
}

function Export-InstallerDiagnostics {
    param(
        [Parameter(Mandatory)][string]$Reason,
        [System.Diagnostics.Process]$InstallerProcess = $null
    )

    Write-TraceEvent -Phase "installer_diagnostics_start" -Detail @{ reason = $Reason; installerPid = $(if ($InstallerProcess) { $InstallerProcess.Id } else { $null }) }
    Write-DiagnosticFile -Name "interactive-context.json" -Capture {
        [ordered]@{
            reason          = $Reason
            computerName    = $env:COMPUTERNAME
            userName        = $env:USERNAME
            sessionName     = $env:SESSIONNAME
            installerPath   = $InstallerPath
            installerArgs   = $InstallerArguments
            operationName   = $OperationName
            resultPath      = $ResultPath
            tracePath       = $TracePath
            timeoutSeconds  = $TimeoutSeconds
            installerPid    = $(if ($InstallerProcess) { $InstallerProcess.Id } else { $null })
            installerExited = $(if ($InstallerProcess) { $InstallerProcess.HasExited } else { $null })
        }
    }
    Write-DiagnosticFile -Name "interactive-processes.json" -Capture {
        Get-CimInstance Win32_Process |
            Where-Object { $_.Name -match 'covenant|Covenant|powershell|pwsh|dotnet' } |
            Select-Object ProcessId, ParentProcessId, Name, CommandLine, CreationDate
    }
    Write-DiagnosticFile -Name "interactive-windows.json" -Capture {
        Get-Process |
            Where-Object { $_.MainWindowHandle -ne 0 -or $_.ProcessName -match 'covenant|Covenant|powershell|pwsh' } |
            Select-Object Id, ProcessName, MainWindowTitle, MainWindowHandle, StartTime
    }
    Write-DiagnosticFile -Name "interactive-application-events.json" -Capture {
        Get-WinEvent -FilterHashtable @{ LogName = "Application"; StartTime = (Get-Date).AddHours(-2) } -MaxEvents 200 -ErrorAction SilentlyContinue |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
    }
    Write-TraceEvent -Phase "installer_diagnostics_finish" -Detail @{ reason = $Reason }
}

$installer = $null
$startedAt = Get-Date

try {
    if (-not [string]::IsNullOrWhiteSpace($InstallerArgumentsBase64)) {
        $argumentsJson = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($InstallerArgumentsBase64))
        $decodedArguments = ConvertFrom-Json -InputObject $argumentsJson
        $InstallerArguments = @()
        foreach ($argument in $decodedArguments) {
            $InstallerArguments += [string]$argument
        }
    }

    New-Item -ItemType Directory -Force -Path $TracePath | Out-Null
    Write-TraceEvent -Phase "interactive_installer_start" -Detail @{
        installerPath  = $InstallerPath
        installerArgs  = $InstallerArguments
        operationName  = $OperationName
        resultPath     = $ResultPath
        tracePath      = $TracePath
        timeoutSeconds = $TimeoutSeconds
    }

    if (-not (Test-Path -LiteralPath $InstallerPath)) {
        throw "Installer not found: $InstallerPath"
    }

    $resultDir = Split-Path -Parent $ResultPath
    if ($resultDir) {
        New-Item -ItemType Directory -Force -Path $resultDir | Out-Null
    }
    Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue

    $env:COVENANT_SETUP_TRACE_DIR = $TracePath
    Write-TraceEvent -Phase "trace_environment_set" -Detail @{ name = "COVENANT_SETUP_TRACE_DIR"; value = $TracePath }

    $installer = Start-Process -FilePath $InstallerPath -ArgumentList $InstallerArguments -PassThru
    Write-TraceEvent -Phase "installer_process_started" -Detail @{ pid = $installer.Id; operationName = $OperationName }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastPoll = Get-Date "2000-01-01"
    while (-not $installer.HasExited) {
        if ((Get-Date) -ge $deadline) {
            Export-InstallerDiagnostics -Reason "installer_timeout" -InstallerProcess $installer
            Stop-Process -Id $installer.Id -Force -ErrorAction SilentlyContinue
            throw "Installer timed out after $TimeoutSeconds seconds."
        }

        if (((Get-Date) - $lastPoll).TotalSeconds -ge 10) {
            $lastPoll = Get-Date
            $installer.Refresh()
            Write-TraceEvent -Phase "installer_still_running" -Detail @{
                pid            = $installer.Id
                operationName  = $OperationName
                elapsedSeconds = [int]((Get-Date) - $startedAt).TotalSeconds
                responding     = $installer.Responding
                mainWindow     = $installer.MainWindowTitle
            }
        }

        Start-Sleep -Seconds 2
        $installer.Refresh()
    }

    Write-TraceEvent -Phase "installer_process_exited" -Detail @{ pid = $installer.Id; exitCode = $installer.ExitCode; operationName = $OperationName }
    Export-InstallerDiagnostics -Reason "installer_exit" -InstallerProcess $installer

    $result = [ordered]@{
        success      = ($installer.ExitCode -eq 0)
        exitCode     = [int]$installer.ExitCode
        installerPath = $InstallerPath
        installerArgs = $InstallerArguments
        operationName = $OperationName
        tracePath     = $TracePath
        startedAt    = $startedAt.ToString("o")
        finishedAt   = (Get-Date).ToString("o")
    }
    $result | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8

    if ($installer.ExitCode -ne 0) {
        exit $installer.ExitCode
    }
}
catch {
    Write-TraceEvent -Phase "interactive_installer_error" -Detail @{
        error = $_.Exception.Message
        type  = $_.Exception.GetType().FullName
    }
    Export-InstallerDiagnostics -Reason "interactive_installer_error" -InstallerProcess $installer
    $failure = [ordered]@{
        success    = $false
        error      = $_.Exception.Message
        tracePath  = $TracePath
        startedAt  = $startedAt.ToString("o")
        finishedAt = (Get-Date).ToString("o")
    }
    $failure | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    exit 1
}
