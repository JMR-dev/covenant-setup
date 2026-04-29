[CmdletBinding()]
param(
    [string]$Provider = "hyperv",
    [string[]]$Scenarios = @("self-test", "uac", "hklm-registry", "reboot", "bundled-exec"),
    [string]$VmName = $(if ($env:COVENANT_VM_NAME) { $env:COVENANT_VM_NAME } else { "covenant-setup-windows" }),
    [string]$GuestUsername = $(if ($env:COVENANT_WINRM_USERNAME) { $env:COVENANT_WINRM_USERNAME } else { "vagrant" }),
    [string]$GuestPassword = $(if ($env:COVENANT_WINRM_PASSWORD) { $env:COVENANT_WINRM_PASSWORD } else { "vagrant" }),
    [switch]$SkipBuild,
    [switch]$SkipVmBoot,
    [switch]$SkipViewer,
    [switch]$HaltAfter,
    [switch]$DestroyAfter
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Coverage-harness orchestrator. Drives the Vagrant Windows VM through every
# scenario directory under vm\<scenario>\install.toml using the per-scenario
# guest scripts under scripts\windows-vm\coverage\<scenario>.ps1.
#
# All install/uninstall side effects happen INSIDE the VM. The host only:
#   1. Builds covenant-setup.exe (release).
#   2. Stages payload trees per scenario (where the manifest references
#      payload\covenant-setup.exe).
#   3. Boots the VM, uploads the exe + scenarios + guest scripts.
#   4. WinRM-invokes each guest script and aggregates results.
#
# Existing scripts under scripts\windows-vm\coverage\*.ps1 are guest-side and
# already accept -Exe -Manifest -WorkRoot.

function Assert-Command {
    param([Parameter(Mandatory)][string]$Name)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found on PATH: $Name"
    }
}

function Invoke-Tool {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$Arguments = @()
    )
    Write-Host "==> $FilePath $($Arguments -join ' ')"
    Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    & $FilePath @Arguments
    $exitCodeVar = Get-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    $exitCode = if ($exitCodeVar) { [int]$exitCodeVar.Value } elseif ($?) { 0 } else { 1 }
    if ($exitCode -ne 0) {
        throw "Command failed with exit code ${exitCode}: $FilePath $($Arguments -join ' ')"
    }
}

function Invoke-Vagrant {
    param([string[]]$Arguments)
    Invoke-Tool -FilePath "vagrant" -Arguments $Arguments
}

function Invoke-VagrantOutput {
    param([string[]]$Arguments)
    Write-Host "==> vagrant $($Arguments -join ' ')"
    Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    $output = & vagrant @Arguments 2>&1
    $exitCodeVar = Get-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    $exitCode = if ($exitCodeVar) { [int]$exitCodeVar.Value } elseif ($?) { 0 } else { 1 }
    return [pscustomobject]@{ Output = ($output | Out-String); ExitCode = $exitCode }
}

function Open-HyperVViewer {
    param([Parameter(Mandatory)][string]$VmName)
    $vmConnect = Join-Path $env:SystemRoot "System32\vmconnect.exe"
    if (-not (Test-Path -LiteralPath $vmConnect)) { return }
    Start-Process -FilePath $vmConnect -ArgumentList @("localhost", $VmName) | Out-Null
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$releaseExe = Join-Path $repoRoot "target\release\covenant-setup.exe"
$outputRoot = Join-Path $repoRoot "dist\vagrant-coverage"
$summaryPath = Join-Path $outputRoot "summary.json"
$guestRoot = "C:\Users\vagrant\AppData\Local\Temp\covenant-setup-coverage"
$guestExe = Join-Path $guestRoot "bin\covenant-setup.exe"
$guestScriptRoot = Join-Path $guestRoot "scripts"
$guestScenarioRoot = Join-Path $guestRoot "scenarios"
$guestWorkRoot = Join-Path $guestRoot "work"

Assert-Command -Name "cargo"
Assert-Command -Name "vagrant"

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

if (-not $SkipBuild) {
    Invoke-Tool -FilePath "cargo" -Arguments @("build", "--release")
}
if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Release binary not found at $releaseExe"
}

# Stage the payload tree for each scenario manifest that references
# payload\covenant-setup.exe (relative to the manifest dir).
foreach ($scenario in $Scenarios) {
    $manifest = Join-Path $repoRoot "vm\$scenario\install.toml"
    if (-not (Test-Path -LiteralPath $manifest)) {
        throw "Scenario manifest not found: $manifest"
    }
    $payloadDir = Join-Path $repoRoot "vm\$scenario\payload"
    $manifestText = Get-Content -LiteralPath $manifest -Raw
    if ($manifestText -match 'payload\\\\covenant-setup\.exe' -or $manifestText -match 'payload[\\/]covenant-setup\.exe') {
        New-Item -ItemType Directory -Force -Path $payloadDir | Out-Null
        Copy-Item -LiteralPath $releaseExe -Destination (Join-Path $payloadDir "covenant-setup.exe") -Force
    }
}

$results = @()
$hadFailure = $false

try {
    if (-not $SkipVmBoot) {
        Invoke-Vagrant -Arguments @("up", "--provider", $Provider)
    }

    if ($Provider -ieq "hyperv" -and -not $SkipViewer) {
        Open-HyperVViewer -VmName $VmName
    }

    $waitForShellCommand = "for (`$i = 0; `$i -lt 90; `$i++) { if (Get-Process -Name explorer -ErrorAction SilentlyContinue) { exit 0 }; Start-Sleep -Seconds 2 }; Write-Error 'Explorer shell did not start in time.'; exit 1"
    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $waitForShellCommand)

    # Prepare guest layout.
    $prepCommand = @(
        "New-Item -ItemType Directory -Force -Path '$guestRoot' | Out-Null"
        "New-Item -ItemType Directory -Force -Path '$(Join-Path $guestRoot 'bin')' | Out-Null"
        "New-Item -ItemType Directory -Force -Path '$guestScriptRoot' | Out-Null"
        "New-Item -ItemType Directory -Force -Path '$guestScenarioRoot' | Out-Null"
        "New-Item -ItemType Directory -Force -Path '$guestWorkRoot' | Out-Null"
    ) -join "; "
    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $prepCommand)

    # Upload covenant-setup.exe and per-scenario assets.
    Invoke-Vagrant -Arguments @("upload", $releaseExe, $guestExe)

    foreach ($scenario in $Scenarios) {
        $localScenarioDir = Join-Path $repoRoot "vm\$scenario"
        $remoteScenarioDir = Join-Path $guestScenarioRoot $scenario
        Invoke-Vagrant -Arguments @("upload", $localScenarioDir, $remoteScenarioDir)
        $localScript = Join-Path $repoRoot "scripts\windows-vm\coverage\$scenario.ps1"
        if (-not (Test-Path -LiteralPath $localScript)) {
            throw "Scenario script not found: $localScript"
        }
        $remoteScript = Join-Path $guestScriptRoot "$scenario.ps1"
        Invoke-Vagrant -Arguments @("upload", $localScript, $remoteScript)
    }

    # Run each scenario in the guest. Capture exit code and stderr/stdout
    # without throwing so we can record per-scenario status.
    foreach ($scenario in $Scenarios) {
        Write-Host ""
        Write-Host "[coverage] -> $scenario" -ForegroundColor Cyan
        $remoteScript = Join-Path $guestScriptRoot "$scenario.ps1"
        $remoteManifest = Join-Path $guestScenarioRoot "$scenario\install.toml"
        $remoteWork = Join-Path $guestWorkRoot $scenario
        $logRel = "$scenario\guest.log"
        $remoteLog = Join-Path $guestWorkRoot $logRel
        $invokeCommand = @(
            "New-Item -ItemType Directory -Force -Path '$remoteWork' | Out-Null"
            "& '$remoteScript' -Exe '$guestExe' -Manifest '$remoteManifest' -WorkRoot '$remoteWork' *> '$remoteLog'"
            "exit `$LASTEXITCODE"
        ) -join "; "

        $invocation = Invoke-VagrantOutput -Arguments @("winrm", "-s", "powershell", "-c", $invokeCommand)
        $localScenarioOut = Join-Path $outputRoot $scenario
        New-Item -ItemType Directory -Force -Path $localScenarioOut | Out-Null

        # Pull the guest log.
        $logFetch = Invoke-VagrantOutput -Arguments @("winrm", "-s", "powershell", "-c", "if (Test-Path -LiteralPath '$remoteLog') { Get-Content -LiteralPath '$remoteLog' -Raw } else { '__COVENANT_NO_LOG__' }")
        Set-Content -LiteralPath (Join-Path $localScenarioOut "guest.log") -Value $logFetch.Output -Encoding UTF8

        $success = ($invocation.ExitCode -eq 0)
        $results += [pscustomobject]@{ scenario = $scenario; success = $success; exitCode = $invocation.ExitCode }
        if (-not $success) {
            $hadFailure = $true
            Write-Host "[coverage] $scenario FAILED (exit $($invocation.ExitCode))" -ForegroundColor Red
            Write-Host $invocation.Output
        } else {
            Write-Host "[coverage] $scenario OK" -ForegroundColor Green
        }
    }
}
finally {
    $summary = [pscustomobject]@{
        scenarios = $results
        success   = -not $hadFailure
    }
    $summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $summaryPath -Encoding UTF8
    Write-Host ""
    Write-Host "Summary: $summaryPath"
    foreach ($r in $results) {
        $color = if ($r.success) { "Green" } else { "Red" }
        Write-Host ("  {0,-18} success={1}  exit={2}" -f $r.scenario, $r.success, $r.exitCode) -ForegroundColor $color
    }

    if ($DestroyAfter) {
        try { Invoke-Vagrant -Arguments @("destroy", "-f") } catch { Write-Warning $_.Exception.Message }
    } elseif ($HaltAfter) {
        try { Invoke-Vagrant -Arguments @("halt") } catch { Write-Warning $_.Exception.Message }
    }
}

if ($hadFailure) {
    throw "One or more coverage scenarios failed. See $summaryPath."
}
