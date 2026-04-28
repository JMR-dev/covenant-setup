param(
    [string]$TracePath = "C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke\trace",
    [string]$ZipPath = "C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke\trace-abort.zip",
    [string]$TaskNamePrefix = "CovenantSetupSelfInstall"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Continue"

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
    Write-TraceEvent -Phase "abort_diagnostic_file_start" -Detail @{ name = $Name }
    try {
        $value = & $Capture
        $value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding UTF8
        Write-TraceEvent -Phase "abort_diagnostic_file_finish" -Detail @{ name = $Name }
    }
    catch {
        [ordered]@{
            error = $_.Exception.Message
            type  = $_.Exception.GetType().FullName
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $path -Encoding UTF8
        Write-TraceEvent -Phase "abort_diagnostic_file_error" -Detail @{
            name  = $Name
            error = $_.Exception.Message
            type  = $_.Exception.GetType().FullName
        }
    }
}

New-Item -ItemType Directory -Force -Path $TracePath | Out-Null
Write-TraceEvent -Phase "abort_requested"

Write-DiagnosticFile -Name "abort-processes.json" -Capture {
    Get-CimInstance Win32_Process |
        Where-Object {
            $_.Name -match "covenant|Covenant|powershell|pwsh" -or
            $_.CommandLine -match "CovenantSetup|Invoke-InteractiveInstaller|Start-InteractiveSelfInstall"
        } |
        Select-Object ProcessId, ParentProcessId, Name, CommandLine, CreationDate
}
Write-DiagnosticFile -Name "abort-windows.json" -Capture {
    Get-Process |
        Where-Object { $_.MainWindowHandle -ne 0 -or $_.ProcessName -match "covenant|Covenant|powershell|pwsh" } |
        Select-Object Id, ProcessName, MainWindowTitle, MainWindowHandle, StartTime
}
Write-DiagnosticFile -Name "abort-scheduled-tasks.json" -Capture {
    Get-ScheduledTask -TaskName "$TaskNamePrefix*" -ErrorAction SilentlyContinue |
        Select-Object TaskName, State, TaskPath, Actions, Triggers
}
Write-DiagnosticFile -Name "abort-scheduled-task-info.json" -Capture {
    Get-ScheduledTask -TaskName "$TaskNamePrefix*" -ErrorAction SilentlyContinue |
        ForEach-Object { Get-ScheduledTaskInfo -TaskName $_.TaskName -ErrorAction SilentlyContinue } |
        Select-Object TaskName, LastRunTime, LastTaskResult, NextRunTime, NumberOfMissedRuns
}
Write-DiagnosticFile -Name "abort-application-events.json" -Capture {
    Get-WinEvent -FilterHashtable @{ LogName = "Application"; StartTime = (Get-Date).AddHours(-2) } -MaxEvents 200 -ErrorAction SilentlyContinue |
        Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
}
Write-DiagnosticFile -Name "abort-system-events.json" -Capture {
    Get-WinEvent -FilterHashtable @{ LogName = "System"; StartTime = (Get-Date).AddHours(-2) } -MaxEvents 200 -ErrorAction SilentlyContinue |
        Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
}

Get-Process -Name "covenant-setup-installer", "covenant-setup", "covenant-setup-uninstall", "Covenant.Setup.Ui" -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue

Get-ScheduledTask -TaskName "$TaskNamePrefix*" -ErrorAction SilentlyContinue |
    Stop-ScheduledTask -ErrorAction SilentlyContinue
Get-ScheduledTask -TaskName "$TaskNamePrefix*" -ErrorAction SilentlyContinue |
    Unregister-ScheduledTask -Confirm:$false -ErrorAction SilentlyContinue

$scriptProcesses = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object {
        $_.ProcessId -ne $PID -and
        $_.CommandLine -match "Invoke-InteractiveInstaller|Start-InteractiveSelfInstall"
    }
foreach ($process in $scriptProcesses) {
    Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
}

Write-TraceEvent -Phase "abort_cleanup_complete"

Remove-Item -LiteralPath $ZipPath -Force -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $TracePath "*") -DestinationPath $ZipPath -Force
Write-Output "__COVENANT_TRACE_B64_START__"
[Convert]::ToBase64String([IO.File]::ReadAllBytes($ZipPath))
Write-Output "__COVENANT_TRACE_B64_END__"
