[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$ResultPath,
    [string]$ScriptRoot = $PSScriptRoot,
    [int]$TimeoutSeconds = 600,
    [string]$TaskNamePrefix = "CovenantSetupSelfInstall",
    [string]$TracePath = $(Join-Path (Split-Path -Parent $ResultPath) "trace")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Quote-TaskArgument {
    param([Parameter(Mandatory)][string]$Value)

    return '"' + $Value.Replace('"', '""') + '"'
}

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
    Write-TraceEvent -Phase "diagnostic_file_start" -Detail @{ name = $Name }
    try {
        $value = & $Capture
        $value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $path -Encoding UTF8
        Write-TraceEvent -Phase "diagnostic_file_finish" -Detail @{ name = $Name }
    }
    catch {
        [ordered]@{
            error = $_.Exception.Message
            type  = $_.Exception.GetType().FullName
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $path -Encoding UTF8
        Write-TraceEvent -Phase "diagnostic_file_error" -Detail @{
            name  = $Name
            error = $_.Exception.Message
            type  = $_.Exception.GetType().FullName
        }
    }
}

function Convert-DateTimeForTrace {
    param([object]$Value)

    if ($null -eq $Value) {
        return $null
    }

    if ($Value -is [DateTime] -and $Value -eq [DateTime]::MinValue) {
        return $null
    }

    try {
        return ([DateTime]$Value).ToString("o")
    }
    catch {
        return [string]$Value
    }
}

function Export-SmokeDiagnostics {
    param([Parameter(Mandatory)][string]$Reason)

    Write-TraceEvent -Phase "diagnostics_start" -Detail @{ reason = $Reason }
    New-Item -ItemType Directory -Force -Path $TracePath | Out-Null

    Write-DiagnosticFile -Name "guest-context.json" -Capture {
        [ordered]@{
            reason        = $Reason
            computerName  = $env:COMPUTERNAME
            userName      = $env:USERNAME
            installerPath = $InstallerPath
            resultPath    = $ResultPath
            tracePath     = $TracePath
            taskName      = $taskName
            installRoot   = $installRoot
        }
    }
    Write-DiagnosticFile -Name "processes.json" -Capture {
        Get-CimInstance Win32_Process |
            Where-Object { $_.Name -match 'covenant|Covenant|powershell|pwsh|dotnet' } |
            Select-Object ProcessId, ParentProcessId, Name, CommandLine, CreationDate
    }
    Write-DiagnosticFile -Name "scheduled-task.json" -Capture {
        [ordered]@{
            task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            info = Get-ScheduledTaskInfo -TaskName $taskName -ErrorAction SilentlyContinue
        }
    }
    Write-DiagnosticFile -Name "scheduled-task-events.json" -Capture {
        Get-WinEvent -LogName "Microsoft-Windows-TaskScheduler/Operational" -MaxEvents 300 -ErrorAction SilentlyContinue |
            Where-Object { $_.Message -like "*$taskName*" } |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
    }
    Write-DiagnosticFile -Name "application-events.json" -Capture {
        Get-WinEvent -FilterHashtable @{ LogName = "Application"; StartTime = (Get-Date).AddHours(-2) } -MaxEvents 200 -ErrorAction SilentlyContinue |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
    }
    Write-DiagnosticFile -Name "system-events.json" -Capture {
        Get-WinEvent -FilterHashtable @{ LogName = "System"; StartTime = (Get-Date).AddHours(-2) } -MaxEvents 200 -ErrorAction SilentlyContinue |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message
    }
    Write-DiagnosticFile -Name "install-root-files.json" -Capture {
        if (Test-Path -LiteralPath $installRoot) {
            Get-ChildItem -LiteralPath $installRoot -Force -Recurse |
                Select-Object FullName, Length, LastWriteTimeUtc, Attributes
        }
        else {
            [ordered]@{ exists = $false; path = $installRoot }
        }
    }
    Write-DiagnosticFile -Name "registry-state.json" -Capture {
        if (Test-Path -LiteralPath $registryPath) {
            Get-ItemProperty -LiteralPath $registryPath
        }
        else {
            [ordered]@{ exists = $false; path = $registryPath }
        }
    }
    Write-DiagnosticFile -Name "result-file.json" -Capture {
        if (Test-Path -LiteralPath $ResultPath) {
            Get-Content -LiteralPath $ResultPath -Raw
        }
        else {
            [ordered]@{ exists = $false; path = $ResultPath }
        }
    }
    Write-TraceEvent -Phase "diagnostics_finish" -Detail @{ reason = $Reason }
}

function Invoke-InteractiveOperation {
    param(
        [Parameter(Mandatory)][string]$TaskName,
        [Parameter(Mandatory)][string]$ExecutablePath,
        [Parameter(Mandatory)][string]$RunResultPath,
        [Parameter(Mandatory)][string]$OperationName,
        [string[]]$Arguments = @()
    )

    $script:taskName = $TaskName
    Remove-Item -LiteralPath $RunResultPath -Force -ErrorAction SilentlyContinue

    # The task is started manually below. Keep the trigger far enough out that it
    # cannot fire a second copy while the smoke harness is collecting diagnostics.
    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddDays(1)
    $actionArgumentItems = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", (Quote-TaskArgument -Value $interactiveScript),
        "-InstallerPath", (Quote-TaskArgument -Value $ExecutablePath),
        "-ResultPath", (Quote-TaskArgument -Value $RunResultPath),
        "-TimeoutSeconds", $TimeoutSeconds,
        "-TracePath", (Quote-TaskArgument -Value $TracePath),
        "-OperationName", (Quote-TaskArgument -Value $OperationName)
    )
    if ($Arguments.Count -gt 0) {
        $argumentsJson = ConvertTo-Json -InputObject $Arguments -Compress
        $argumentsBase64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($argumentsJson))
        $actionArgumentItems += "-InstallerArgumentsBase64"
        $actionArgumentItems += $argumentsBase64
    }
    $actionArguments = $actionArgumentItems -join " "
    $action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $actionArguments
    $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable

    try {
        Register-ScheduledTask `
            -TaskName $TaskName `
            -Action $action `
            -Trigger $trigger `
            -Settings $settings `
            -Principal $principal `
            -Force | Out-Null
        Write-TraceEvent -Phase "scheduled_task_registered" -Detail @{
            taskName        = $TaskName
            operationName   = $OperationName
            executablePath  = $ExecutablePath
            executableArgs  = $Arguments
            actionArguments = $actionArguments
            runResultPath   = $RunResultPath
        }

        Start-ScheduledTask -TaskName $TaskName
        Write-TraceEvent -Phase "scheduled_task_started" -Detail @{ taskName = $TaskName; operationName = $OperationName }

        $deadline = (Get-Date).AddSeconds($TimeoutSeconds + 120)
        $lastPoll = Get-Date "2000-01-01"
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $RunResultPath) {
                Write-TraceEvent -Phase "result_file_observed" -Detail @{
                    taskName      = $TaskName
                    operationName = $OperationName
                    runResultPath = $RunResultPath
                }
                break
            }

            if (((Get-Date) - $lastPoll).TotalSeconds -ge 10) {
                $lastPoll = Get-Date
                $taskInfo = Get-ScheduledTaskInfo -TaskName $TaskName -ErrorAction SilentlyContinue
                $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
                Write-TraceEvent -Phase "waiting_for_interactive_task" -Detail @{
                    taskName       = $TaskName
                    operationName  = $OperationName
                    state          = $(if ($task) { $task.State.ToString() } else { $null })
                    lastRunTime    = $(if ($taskInfo) { Convert-DateTimeForTrace -Value $taskInfo.LastRunTime } else { $null })
                    lastTaskResult = $(if ($taskInfo) { $taskInfo.LastTaskResult } else { $null })
                    nextRunTime    = $(if ($taskInfo) { Convert-DateTimeForTrace -Value $taskInfo.NextRunTime } else { $null })
                }
            }

            Start-Sleep -Seconds 2
        }

        if (-not (Test-Path -LiteralPath $RunResultPath)) {
            Export-SmokeDiagnostics -Reason "${OperationName}_task_timeout"
            $taskInfo = Get-ScheduledTaskInfo -TaskName $TaskName
            throw "Timed out waiting for $OperationName interactive task. LastTaskResult=$($taskInfo.LastTaskResult)"
        }

        $runResult = Get-Content -LiteralPath $RunResultPath -Raw | ConvertFrom-Json
        Write-TraceEvent -Phase "interactive_task_result_read" -Detail @{
            taskName      = $TaskName
            operationName = $OperationName
            result        = $runResult
        }
        if (-not $runResult.success) {
            Export-SmokeDiagnostics -Reason "${OperationName}_task_failed"
            throw "Interactive $OperationName task failed: $($runResult.error)"
        }

        return $runResult
    }
    finally {
        Write-TraceEvent -Phase "scheduled_task_unregister" -Detail @{ taskName = $TaskName; operationName = $OperationName }
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    }
}

function Wait-ForUninstallCleanup {
    param([int]$TimeoutSeconds = 60)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastPoll = Get-Date "2000-01-01"
    while ((Get-Date) -lt $deadline) {
        $remaining = [ordered]@{
            installRoot = Test-Path -LiteralPath $installRoot
            installedExe = Test-Path -LiteralPath $installedExe
            journalPath = Test-Path -LiteralPath $journalPath
            uninstallExe = Test-Path -LiteralPath $uninstallExe
            shortcutPath = Test-Path -LiteralPath $shortcutPath
            registryPath = Test-Path -LiteralPath $registryPath
            uninstallRegistryPath = Test-Path -LiteralPath $uninstallRegistryPath
        }

        if (-not ($remaining.installRoot -or $remaining.installedExe -or $remaining.journalPath -or $remaining.uninstallExe -or $remaining.shortcutPath -or $remaining.registryPath -or $remaining.uninstallRegistryPath)) {
            Write-TraceEvent -Phase "uninstall_cleanup_observed" -Detail $remaining
            return
        }

        if (((Get-Date) - $lastPoll).TotalSeconds -ge 5) {
            $lastPoll = Get-Date
            Write-TraceEvent -Phase "waiting_for_uninstall_cleanup" -Detail $remaining
        }

        Start-Sleep -Seconds 1
    }

    throw "Uninstall cleanup did not complete within $TimeoutSeconds seconds."
}

$installRoot = Join-Path $env:LOCALAPPDATA "CovenantSetupSelfTest"
$shortcutPath = Join-Path ([Environment]::GetFolderPath("Desktop")) "CovenantSetupSelfTest.lnk"
$registryPath = "HKCU:\Software\CovenantSetupSelfTest"
$journalPath = Join-Path $installRoot "journal.json"
$installedExe = Join-Path $installRoot "bin\covenant-setup.exe"
$uninstallExe = Join-Path $installRoot "covenant-setup-uninstall.exe"
$uninstallRegistryPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Covenant_Setup_Self_Test"
$taskStamp = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$installTaskName = "{0}-Install-{1}" -f $TaskNamePrefix, $taskStamp
$uninstallTaskName = "{0}-Uninstall-{1}" -f $TaskNamePrefix, $taskStamp
$taskName = $installTaskName
$interactiveScript = Join-Path $ScriptRoot "Invoke-InteractiveInstaller.ps1"
$installRunResultPath = $null
$uninstallRunResultPath = $null

try {
    Remove-Item -LiteralPath $TracePath -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $TracePath | Out-Null
    Write-TraceEvent -Phase "self_install_start" -Detail @{
        installerPath  = $InstallerPath
        resultPath     = $ResultPath
        tracePath      = $TracePath
        timeoutSeconds = $TimeoutSeconds
    }

    # Install VC++ Redistributable if missing
    $vcredistInstalled = $false
    $vcredistKey = "HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64"
    if (Test-Path -Path $vcredistKey) {
        $vcredistVal = Get-ItemProperty -Path $vcredistKey -Name "Installed" -ErrorAction SilentlyContinue
        if ($vcredistVal -and $vcredistVal.Installed -eq 1) {
            $vcredistInstalled = $true
        }
    }

    if (-not $vcredistInstalled) {
        Write-TraceEvent -Phase "vcredist_install_start"
        try {
            $vcredistPath = Join-Path $env:TEMP "vc_redist.x64.exe"
            Remove-Item -Path $vcredistPath -Force -ErrorAction SilentlyContinue
            Write-Host "Downloading VC++ Redistributable..."
            $oldProgress = $ProgressPreference
            $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vc_redist.x64.exe" -OutFile $vcredistPath
            $ProgressPreference = $oldProgress
            
            Write-Host "Installing VC++ Redistributable..."
            $proc = Start-Process -FilePath $vcredistPath -ArgumentList "/install", "/quiet", "/norestart" -PassThru -Wait
            if ($proc.ExitCode -ne 0 -and $proc.ExitCode -ne 3010) { # 3010 is success but reboot required
                throw "vc_redist installation failed with exit code $($proc.ExitCode)"
            }
            Write-TraceEvent -Phase "vcredist_install_success" -Detail @{ exitCode = $proc.ExitCode }
        }
        catch {
            Write-TraceEvent -Phase "vcredist_install_error" -Detail @{ error = $_.Exception.Message }
            Write-Warning "Failed to install VC++ Redistributable: $($_.Exception.Message)"
        }
    }
    else {
        Write-TraceEvent -Phase "vcredist_already_installed"
    }

    if (-not (Test-Path -LiteralPath $InstallerPath)) {
        throw "Installer not found in guest: $InstallerPath"
    }

    if (-not (Test-Path -LiteralPath $interactiveScript)) {
        throw "Interactive installer script not found in guest: $interactiveScript"
    }

    $resultDir = Split-Path -Parent $ResultPath
    if ($resultDir) {
        New-Item -ItemType Directory -Force -Path $resultDir | Out-Null
    }
    Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue
    $installRunResultPath = Join-Path $resultDir "install-run-result.json"
    $uninstallRunResultPath = Join-Path $resultDir "uninstall-run-result.json"
    Remove-Item -LiteralPath $installRunResultPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $uninstallRunResultPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $registryPath -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $uninstallRegistryPath -Recurse -Force -ErrorAction SilentlyContinue
    Write-TraceEvent -Phase "self_install_cleaned_previous_state"

    $installRunResult = Invoke-InteractiveOperation `
        -TaskName $installTaskName `
        -ExecutablePath $InstallerPath `
        -RunResultPath $installRunResultPath `
        -OperationName "install" `
        -Arguments @("--headed", "--automation")

    Write-TraceEvent -Phase "verification_start"
    if (-not (Test-Path -LiteralPath $installedExe)) {
        throw "Installed executable missing: $installedExe"
    }
    if (-not (Test-Path -LiteralPath $journalPath)) {
        throw "Journal missing: $journalPath"
    }
    if (-not (Test-Path -LiteralPath $uninstallExe)) {
        throw "Installed uninstaller missing: $uninstallExe"
    }
    if (-not (Test-Path -LiteralPath $shortcutPath)) {
        throw "Desktop shortcut missing: $shortcutPath"
    }
    if (-not (Test-Path -LiteralPath $registryPath)) {
        throw "Registry key missing: $registryPath"
    }

    $installRootValue = Get-ItemPropertyValue -Path $registryPath -Name "InstallRoot"
    if ($installRootValue -ne $installRoot) {
        throw "Unexpected registry InstallRoot value: $installRootValue"
    }

    $journal = Get-Content -LiteralPath $journalPath -Raw | ConvertFrom-Json
    $journalActionTypes = @($journal.actions | ForEach-Object { $_.type })
    if ($journal.app_name -ne "Covenant Setup Self Test") {
        throw "Unexpected journal app name: $($journal.app_name)"
    }
    if ($journalActionTypes -notcontains "copy_file") {
        throw "Journal did not record the packaged payload copy."
    }

    $uninstallRunResult = Invoke-InteractiveOperation `
        -TaskName $uninstallTaskName `
        -ExecutablePath $uninstallExe `
        -RunResultPath $uninstallRunResultPath `
        -OperationName "uninstall" `
        -Arguments @("--headed", "--automation", "uninstall", $journalPath)

    Write-TraceEvent -Phase "uninstall_verification_start"
    Wait-ForUninstallCleanup -TimeoutSeconds 60
    if (Test-Path -LiteralPath $installedExe) {
        throw "Installed executable still exists after uninstall: $installedExe"
    }
    if (Test-Path -LiteralPath $journalPath) {
        throw "Journal still exists after uninstall: $journalPath"
    }
    if (Test-Path -LiteralPath $uninstallExe) {
        throw "Installed uninstaller still exists after uninstall: $uninstallExe"
    }
    if (Test-Path -LiteralPath $shortcutPath) {
        throw "Desktop shortcut still exists after uninstall: $shortcutPath"
    }
    if (Test-Path -LiteralPath $registryPath) {
        throw "Registry key still exists after uninstall: $registryPath"
    }
    if (Test-Path -LiteralPath $uninstallRegistryPath) {
        throw "Installed Apps registry key still exists after uninstall: $uninstallRegistryPath"
    }
    if (Test-Path -LiteralPath $installRoot) {
        throw "Install root still exists after uninstall: $installRoot"
    }

    $verification = [ordered]@{
        success            = $true
        exitCode           = 0
        installExitCode    = [int]$installRunResult.exitCode
        uninstallExitCode  = [int]$uninstallRunResult.exitCode
        installerPath      = $InstallerPath
        installRoot        = $installRoot
        installedExe       = $installedExe
        journalPath        = $journalPath
        uninstallExe       = $uninstallExe
        shortcutPath       = $shortcutPath
        registryPath       = $registryPath
        uninstallRegistryPath = $uninstallRegistryPath
        tracePath          = $TracePath
        installRunResultPath = $installRunResultPath
        uninstallRunResultPath = $uninstallRunResultPath
        journalActionTypes = $journalActionTypes
        installTaskName    = $installTaskName
        uninstallTaskName  = $uninstallTaskName
        installStartedAt   = $installRunResult.startedAt
        installFinishedAt  = $installRunResult.finishedAt
        uninstallStartedAt = $uninstallRunResult.startedAt
        uninstallFinishedAt = $uninstallRunResult.finishedAt
        uninstallVerified  = $true
    }
    $verification | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    Write-TraceEvent -Phase "self_install_success" -Detail $verification
}
catch {
    Write-TraceEvent -Phase "self_install_error" -Detail @{
        error = $_.Exception.Message
        type  = $_.Exception.GetType().FullName
    }
    Export-SmokeDiagnostics -Reason "self_install_error"
    $failure = [ordered]@{
        success                = $false
        error                  = $_.Exception.Message
        taskName               = $taskName
        installTaskName        = $installTaskName
        uninstallTaskName      = $uninstallTaskName
        installRoot            = $installRoot
        resultPath             = $ResultPath
        installRunResultPath   = $installRunResultPath
        uninstallRunResultPath = $uninstallRunResultPath
        tracePath              = $TracePath
    }
    $failure | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    exit 1
}
finally {
    foreach ($taskToRemove in @($installTaskName, $uninstallTaskName)) {
        if ($taskToRemove) {
            Unregister-ScheduledTask -TaskName $taskToRemove -Confirm:$false -ErrorAction SilentlyContinue
        }
    }
}
