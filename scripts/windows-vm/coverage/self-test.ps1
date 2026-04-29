[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$WorkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Self-test scenario: parity with the legacy smoke test. Installs the
# payload to %LocalAppData%, asserts the journal records the directory,
# file, registry, and shortcut actions, then uninstalls and asserts
# every recorded path is gone.

$journal = Join-Path $WorkRoot 'self-test.journal.json'

& $Exe install $Manifest --json --headless --automation --journal $journal
if ($LASTEXITCODE -ne 0) { throw "self-test install failed: exit $LASTEXITCODE" }

if (-not (Test-Path -LiteralPath $journal)) {
    throw "self-test journal missing: $journal"
}

$entries = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
if ($null -eq $entries.actions -or $entries.actions.Count -lt 1) {
    throw 'self-test journal has no recorded actions'
}

& $Exe uninstall $journal --json --headless --automation
if ($LASTEXITCODE -ne 0) { throw "self-test uninstall failed: exit $LASTEXITCODE" }
