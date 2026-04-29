[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$WorkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# HKLM registry scenario: writes an HKLM key so the requires_admin
# decision is forced via the registry-root path independent of file
# locations. Without --elevate the install must fail; with --elevate
# it must succeed and the journal must record the HKLM write.

$journal = Join-Path $WorkRoot 'hklm-registry.journal.json'

$noElevate = & $Exe install $Manifest --json --headless --automation --journal $journal 2>&1
if ($LASTEXITCODE -eq 0) {
    throw 'hklm-registry scenario: install without --elevate unexpectedly succeeded'
}
if (-not ($noElevate -match 'Elevation required')) {
    throw "hklm-registry scenario: missing 'Elevation required' message; got: $noElevate"
}

& $Exe install $Manifest --json --headless --automation --elevate --journal $journal
if ($LASTEXITCODE -ne 0) {
    throw "hklm-registry scenario: elevated install failed: exit $LASTEXITCODE"
}

$entries = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
$hklmHit = $entries.actions | Where-Object { $_.type -eq 'write_registry' -and $_.root -eq 'hklm' }
if (-not $hklmHit) {
    throw 'hklm-registry scenario: journal did not record an HKLM write_registry action'
}

& $Exe uninstall $journal --json --headless --automation --elevate
if ($LASTEXITCODE -ne 0) {
    throw "hklm-registry scenario: elevated uninstall failed: exit $LASTEXITCODE"
}
