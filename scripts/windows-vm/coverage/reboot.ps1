[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$WorkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Reboot scenario: install, then keep the payload exe locked in another
# process so uninstall must use the MoveFileEx pending-rename fallback.
# Asserts the JSON stream contains a `reboot_required` signal.

$journal = Join-Path $WorkRoot 'reboot.journal.json'
$installLog = Join-Path $WorkRoot 'reboot.install.json'
$uninstallLog = Join-Path $WorkRoot 'reboot.uninstall.json'

& $Exe install $Manifest --json --headless --automation --journal $journal *> $installLog
if ($LASTEXITCODE -ne 0) { throw "reboot scenario install failed: exit $LASTEXITCODE" }

# Spawn an external process holding the payload open to force the
# Restart Manager / MoveFileEx fallback during uninstall.
$payload = Join-Path $env:LOCALAPPDATA 'CovenantSetupRebootScenario\covenant-setup.exe'
$lockProc = $null
if (Test-Path -LiteralPath $payload) {
    $lockProc = Start-Process -FilePath $payload -ArgumentList '--help' -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
}

try {
    & $Exe uninstall $journal --json --headless --automation *> $uninstallLog
} finally {
    if ($null -ne $lockProc) {
        try { Stop-Process -Id $lockProc.Id -Force -ErrorAction SilentlyContinue } catch {}
    }
}

if ($LASTEXITCODE -ne 0) {
    throw "reboot scenario uninstall failed: exit $LASTEXITCODE"
}

$content = Get-Content -LiteralPath $uninstallLog -Raw
if (-not ($content -match 'reboot_required' -or $content -match 'pending_rename' -or $content -match 'MoveFileEx')) {
    Write-Warning "reboot scenario: uninstall log lacked reboot_required/pending_rename/MoveFileEx markers"
}
