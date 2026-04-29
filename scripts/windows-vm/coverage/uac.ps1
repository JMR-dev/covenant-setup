[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$WorkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# UAC scenario: target ProgramFiles to force requires_admin = true.
# Without --elevate the install must fail fast with the documented
# "Elevation required" message; with --elevate it must complete (when
# run inside an elevated WinRM session) or trigger the relaunch path.

$journal = Join-Path $WorkRoot 'uac.journal.json'

# 1. Without --elevate the runner expects exit-code != 0 and an error
#    message containing "Elevation required".
$noElevate = & $Exe install $Manifest --json --headless --automation --journal $journal 2>&1
if ($LASTEXITCODE -eq 0) {
    throw 'uac scenario: install without --elevate unexpectedly succeeded'
}
if (-not ($noElevate -match 'Elevation required')) {
    throw "uac scenario: missing 'Elevation required' message; got: $noElevate"
}

# 2. With --elevate the install must succeed when invoked from an
#    already-elevated session (Vagrant WinRM provisioner is elevated).
& $Exe install $Manifest --json --headless --automation --elevate --journal $journal
if ($LASTEXITCODE -ne 0) {
    throw "uac scenario: elevated install failed: exit $LASTEXITCODE"
}

& $Exe uninstall $journal --json --headless --automation --elevate
if ($LASTEXITCODE -ne 0) {
    throw "uac scenario: elevated uninstall failed: exit $LASTEXITCODE"
}
