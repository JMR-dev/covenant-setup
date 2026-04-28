[CmdletBinding()]
param(
    [string]$Provider = "hyperv",
    [string]$ManifestPath = "vm\self-test\install.toml",
    [string]$OutputRoot = "dist\vagrant-self-test",
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

function Assert-Command {
    param([Parameter(Mandatory)][string]$Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found on PATH: $Name"
    }
}

function Resolve-RepoPath {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }

    return Join-Path $RepoRoot $Path
}

function Convert-ToSingleQuotedPowerShellLiteral {
    param([Parameter(Mandatory)][string]$Value)

    return "'" + $Value.Replace("'", "''") + "'"
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

function Save-GuestTraceBundle {
    param(
        [Parameter(Mandatory)][string]$GuestTracePath,
        [Parameter(Mandatory)][string]$GuestZipPath,
        [Parameter(Mandatory)][string]$LocalTracePath
    )

    try {
        New-Item -ItemType Directory -Force -Path $LocalTracePath | Out-Null
        $guestTraceLiteral = Convert-ToSingleQuotedPowerShellLiteral -Value $GuestTracePath
        $guestZipLiteral = Convert-ToSingleQuotedPowerShellLiteral -Value $GuestZipPath
        $command = @(
            "`$tracePath = $guestTraceLiteral"
            "`$zipPath = $guestZipLiteral"
            "if (-not (Test-Path -LiteralPath `$tracePath)) { Write-Output '__COVENANT_TRACE_EMPTY__'; exit 0 }"
            "New-Item -ItemType File -Force -Path (Join-Path `$tracePath '.keep') | Out-Null"
            "Remove-Item -LiteralPath `$zipPath -Force -ErrorAction SilentlyContinue"
            "Compress-Archive -Path (Join-Path `$tracePath '*') -DestinationPath `$zipPath -Force"
            "Write-Output '__COVENANT_TRACE_B64_START__'"
            "[Convert]::ToBase64String([IO.File]::ReadAllBytes(`$zipPath))"
            "Write-Output '__COVENANT_TRACE_B64_END__'"
        ) -join "; "

        $output = Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $command)
        $lines = @($output | ForEach-Object { $_.ToString().Trim() })
        if ($lines -contains "__COVENANT_TRACE_EMPTY__") {
            Write-Warning "Guest trace path did not exist: $GuestTracePath"
            return
        }

        $start = [Array]::IndexOf($lines, "__COVENANT_TRACE_B64_START__")
        $end = [Array]::IndexOf($lines, "__COVENANT_TRACE_B64_END__")
        if ($start -lt 0 -or $end -le $start) {
            Write-Warning "Guest trace bundle markers were not found in WinRM output."
            return
        }

        $base64 = (($lines[($start + 1)..($end - 1)]) -join "").Trim()
        if (-not $base64) {
            Write-Warning "Guest trace bundle was empty."
            return
        }

        $zipPath = Join-Path $LocalTracePath "guest-trace.zip"
        [IO.File]::WriteAllBytes($zipPath, [Convert]::FromBase64String($base64))
        Expand-Archive -LiteralPath $zipPath -DestinationPath $LocalTracePath -Force
        Write-Host "Trace bundle: $LocalTracePath"
    }
    catch {
        Write-Warning "Failed to collect guest trace bundle: $($_.Exception.Message)"
    }
}

function Open-HyperVViewer {
    param([Parameter(Mandatory)][string]$VmName)

    $vmConnect = Join-Path $env:SystemRoot "System32\vmconnect.exe"
    if (-not (Test-Path -LiteralPath $vmConnect)) {
        Write-Warning "Hyper-V viewer not found at $vmConnect"
        return
    }

    Write-Host "==> $vmConnect localhost $VmName"
    Start-Process -FilePath $vmConnect -ArgumentList @("localhost", $VmName) | Out-Null
}

function Invoke-Process {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$Arguments = @()
    )

    Write-Host "==> $FilePath $($Arguments -join ' ')"
    $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Command failed with exit code $($process.ExitCode): $FilePath $($Arguments -join ' ')"
    }
}

function Assert-PackagedInstallerBundle {
    param([Parameter(Mandatory)][string]$InstallerPath)

    $magic = [Text.Encoding]::ASCII.GetBytes("COVENANT_SETUP_BUNDLE_V1")
    $stream = [IO.File]::Open(
        $InstallerPath,
        [IO.FileMode]::Open,
        [IO.FileAccess]::Read,
        [IO.FileShare]::ReadWrite)
    try {
        if ($stream.Length -lt $magic.Length) {
            throw "Packaged installer is too small to contain the embedded bundle marker: $InstallerPath"
        }

        $null = $stream.Seek(-1 * $magic.Length, [IO.SeekOrigin]::End)
        $actual = [byte[]]::new($magic.Length)
        $read = $stream.Read($actual, 0, $actual.Length)
        if ($read -ne $magic.Length) {
            throw "Could not read embedded bundle marker from packaged installer: $InstallerPath"
        }

        for ($i = 0; $i -lt $magic.Length; $i++) {
            if ($actual[$i] -ne $magic[$i]) {
                throw "Packaged installer is missing the embedded bundle marker: $InstallerPath"
            }
        }
    }
    finally {
        $stream.Dispose()
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$releaseExe = Join-Path $repoRoot "target\release\covenant-setup.exe"
$payloadRoot = Join-Path $repoRoot "vm\self-test\payload"
$stagedPayload = Join-Path $payloadRoot "covenant-setup.exe"
$manifestPathAbs = Resolve-RepoPath -RepoRoot $repoRoot -Path $ManifestPath
$outputRootAbs = Resolve-RepoPath -RepoRoot $repoRoot -Path $OutputRoot
$installerPath = Join-Path $outputRootAbs "covenant-setup-installer.exe"
$resultPath = Join-Path $outputRootAbs "guest-result.json"
$traceRootAbs = Join-Path $outputRootAbs "trace"
$guestRoot = "C:\Users\vagrant\AppData\Local\Temp\covenant-setup-smoke"
$guestInstallerPath = Join-Path $guestRoot "covenant-setup-installer.exe"
$guestResultPath = Join-Path $guestRoot "guest-result.json"
$guestTraceRoot = Join-Path $guestRoot "trace"
$guestTraceZipPath = Join-Path $guestRoot "trace.zip"
$guestScriptRoot = Join-Path $guestRoot "scripts"
$guestCommand = "& '$guestScriptRoot\Start-InteractiveSelfInstall.ps1' -InstallerPath '$guestInstallerPath' -ResultPath '$guestResultPath' -ScriptRoot '$guestScriptRoot' -TracePath '$guestTraceRoot'"

Assert-Command -Name "cargo"
Assert-Command -Name "dotnet"
Assert-Command -Name "vagrant"

New-Item -ItemType Directory -Force -Path $payloadRoot | Out-Null
New-Item -ItemType Directory -Force -Path $outputRootAbs | Out-Null
Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $traceRootAbs -Recurse -Force -ErrorAction SilentlyContinue

try {
    if (-not $SkipBuild) {
        Invoke-Tool -FilePath "cargo" -Arguments @("build", "--release")
    }

    if (-not (Test-Path -LiteralPath $releaseExe)) {
        throw "Release binary not found at $releaseExe"
    }

    if (-not (Test-Path -LiteralPath $manifestPathAbs)) {
        throw "Self-test manifest not found at $manifestPathAbs"
    }

    Copy-Item -LiteralPath $releaseExe -Destination $stagedPayload -Force
    Invoke-Process -FilePath $releaseExe -Arguments @("package", $manifestPathAbs, "--output", $outputRootAbs)

    if (-not (Test-Path -LiteralPath $installerPath)) {
        throw "Packaged installer not found at $installerPath"
    }
    Assert-PackagedInstallerBundle -InstallerPath $installerPath

    if (-not $SkipVmBoot) {
        Invoke-Vagrant -Arguments @("up", "--provider", $Provider)

        $guestUsernameLiteral = Convert-ToSingleQuotedPowerShellLiteral -Value $GuestUsername
        $guestPasswordLiteral = Convert-ToSingleQuotedPowerShellLiteral -Value $GuestPassword
        $enableAutoLogonCommand = @(
            '$winlogon = ''HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'''
            "New-ItemProperty -Path `$winlogon -Name 'AutoAdminLogon' -PropertyType String -Value '1' -Force | Out-Null"
            "New-ItemProperty -Path `$winlogon -Name 'ForceAutoLogon' -PropertyType String -Value '1' -Force | Out-Null"
            "New-ItemProperty -Path `$winlogon -Name 'DefaultUserName' -PropertyType String -Value $guestUsernameLiteral -Force | Out-Null"
            "New-ItemProperty -Path `$winlogon -Name 'DefaultPassword' -PropertyType String -Value $guestPasswordLiteral -Force | Out-Null"
            "New-ItemProperty -Path `$winlogon -Name 'DefaultDomainName' -PropertyType String -Value `$env:COMPUTERNAME -Force | Out-Null"
        ) -join "; "
        Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $enableAutoLogonCommand)
        Invoke-Vagrant -Arguments @("reload")
    }

    if ($Provider -ieq "hyperv" -and -not $SkipViewer) {
        Open-HyperVViewer -VmName $VmName
    }

    $waitForShellCommand = "for (`$i = 0; `$i -lt 90; `$i++) { if (Get-Process -Name explorer -ErrorAction SilentlyContinue) { exit 0 }; Start-Sleep -Seconds 2 }; Write-Error 'Explorer shell did not start in time.'; exit 1"
    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $waitForShellCommand)

    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", "New-Item -ItemType Directory -Force -Path '$guestRoot' | Out-Null; New-Item -ItemType Directory -Force -Path '$guestScriptRoot' | Out-Null")
    $clearGuestTraceCommand = @(
        "try {"
        "if (Test-Path -LiteralPath '$guestTraceRoot') { Remove-Item -LiteralPath '$guestTraceRoot' -Recurse -Force -ErrorAction SilentlyContinue }"
        "if (Test-Path -LiteralPath '$guestTraceZipPath') { Remove-Item -LiteralPath '$guestTraceZipPath' -Force -ErrorAction SilentlyContinue }"
        "} catch { Write-Warning `$_.Exception.Message }"
        "exit 0"
    ) -join "; "
    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $clearGuestTraceCommand)
    Invoke-Vagrant -Arguments @("upload", $installerPath, $guestInstallerPath)
    Invoke-Vagrant -Arguments @("upload", (Join-Path $repoRoot "scripts\windows-vm\Invoke-InteractiveInstaller.ps1"), (Join-Path $guestScriptRoot "Invoke-InteractiveInstaller.ps1"))
    Invoke-Vagrant -Arguments @("upload", (Join-Path $repoRoot "scripts\windows-vm\Start-InteractiveSelfInstall.ps1"), (Join-Path $guestScriptRoot "Start-InteractiveSelfInstall.ps1"))

    Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", $guestCommand)

    $resultJson = Invoke-Vagrant -Arguments @("winrm", "-s", "powershell", "-c", "Get-Content -LiteralPath '$guestResultPath' -Raw")
    Set-Content -LiteralPath $resultPath -Value $resultJson -Encoding UTF8

    if (-not (Test-Path -LiteralPath $resultPath)) {
        throw "Guest smoke-test result file was not written: $resultPath"
    }

    $result = $resultJson | ConvertFrom-Json
    if (-not $result.success) {
        throw "Guest reported a failed smoke test: $($result.error)"
    }

    Write-Host ""
    Write-Host "Smoke test passed."
    Write-Host "Installer:   $installerPath"
    Write-Host "Result JSON: $resultPath"
    Write-Host "Trace:       $traceRootAbs"
    Write-Host "InstallRoot: $($result.installRoot) (installed, then removed)"
    Write-Host "Uninstall:   exit $($result.uninstallExitCode), verified=$($result.uninstallVerified)"
}
finally {
    Save-GuestTraceBundle -GuestTracePath $guestTraceRoot -GuestZipPath $guestTraceZipPath -LocalTracePath $traceRootAbs

    if ($DestroyAfter) {
        try {
            Invoke-Vagrant -Arguments @("destroy", "-f")
        }
        catch {
            Write-Warning "Failed to destroy VM after smoke test: $($_.Exception.Message)"
        }
    }
    elseif ($HaltAfter) {
        try {
            Invoke-Vagrant -Arguments @("halt")
        }
        catch {
            Write-Warning "Failed to halt VM after smoke test: $($_.Exception.Message)"
        }
    }
}
