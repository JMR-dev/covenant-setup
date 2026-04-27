[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$ResultPath,
    [string]$ScriptRoot = $PSScriptRoot,
    [int]$TimeoutSeconds = 600,
    [string]$TaskNamePrefix = "CovenantSetupSelfInstall"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Quote-TaskArgument {
    param([Parameter(Mandatory)][string]$Value)

    return '"' + $Value.Replace('"', '""') + '"'
}

$installRoot = Join-Path $env:LOCALAPPDATA "CovenantSetupSelfTest"
$shortcutPath = Join-Path ([Environment]::GetFolderPath("Desktop")) "Covenant Setup Self Test.lnk"
$registryPath = "HKCU:\Software\CovenantSetupSelfTest"
$journalPath = Join-Path $installRoot "journal.json"
$installedExe = Join-Path $installRoot "bin\covenant-setup.exe"
$uninstallExe = Join-Path $installRoot "covenant-setup-uninstall.exe"
$taskName = "{0}-{1}" -f $TaskNamePrefix, ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
$interactiveScript = Join-Path $ScriptRoot "Invoke-InteractiveInstaller.ps1"

try {
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
    Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $registryPath -Recurse -Force -ErrorAction SilentlyContinue

    $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1)
    $actionArguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", (Quote-TaskArgument -Value $interactiveScript),
        "-InstallerPath", (Quote-TaskArgument -Value $InstallerPath),
        "-ResultPath", (Quote-TaskArgument -Value $ResultPath),
        "-TimeoutSeconds", $TimeoutSeconds
    ) -join " "
    $action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $actionArguments
    $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable

    Register-ScheduledTask `
        -TaskName $taskName `
        -Action $action `
        -Trigger $trigger `
        -Settings $settings `
        -Principal $principal `
        -Force | Out-Null

    Start-ScheduledTask -TaskName $taskName

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds + 120)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $ResultPath) {
            break
        }
        Start-Sleep -Seconds 2
    }

    if (-not (Test-Path -LiteralPath $ResultPath)) {
        $taskInfo = Get-ScheduledTaskInfo -TaskName $taskName
        throw "Timed out waiting for interactive installer task. LastTaskResult=$($taskInfo.LastTaskResult)"
    }

    $runResult = Get-Content -LiteralPath $ResultPath -Raw | ConvertFrom-Json
    if (-not $runResult.success) {
        throw "Interactive installer task failed: $($runResult.error)"
    }

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

    $verification = [ordered]@{
        success          = $true
        exitCode         = [int]$runResult.exitCode
        installerPath    = $InstallerPath
        installRoot      = $installRoot
        installedExe     = $installedExe
        journalPath      = $journalPath
        uninstallExe     = $uninstallExe
        shortcutPath     = $shortcutPath
        registryPath     = $registryPath
        journalActionTypes = $journalActionTypes
        taskName         = $taskName
        startedAt        = $runResult.startedAt
        finishedAt       = $runResult.finishedAt
    }
    $verification | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
}
catch {
    $failure = [ordered]@{
        success   = $false
        error     = $_.Exception.Message
        taskName  = $taskName
        installRoot = $installRoot
        resultPath = $ResultPath
    }
    $failure | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    exit 1
}
finally {
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
}
