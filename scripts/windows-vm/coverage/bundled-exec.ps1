[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][string]$WorkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Bundled-exec scenario: package the manifest into a single-file installer
# and invoke that bundled exe with no subcommand (which triggers the
# embedded-bundle probe path). The runner then asserts the bundled run
# produced a journal whose actions match the source manifest.

$packageDir = Join-Path $WorkRoot 'bundled-exec-package'
$null = New-Item -ItemType Directory -Force -Path $packageDir

& $Exe package $Manifest --output $packageDir
if ($LASTEXITCODE -ne 0) { throw "bundled-exec package failed: exit $LASTEXITCODE" }

$bundle = Get-ChildItem -LiteralPath $packageDir -Filter '*.exe' | Select-Object -First 1
if ($null -eq $bundle) {
    throw "bundled-exec scenario: no .exe produced under $packageDir"
}

$journal = Join-Path $WorkRoot 'bundled-exec.journal.json'

& $bundle.FullName --json --headless --automation install --journal $journal
if ($LASTEXITCODE -ne 0) {
    throw "bundled-exec install failed: exit $LASTEXITCODE"
}

$entries = Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
if ($null -eq $entries.actions -or $entries.actions.Count -lt 1) {
    throw 'bundled-exec scenario: bundled install journal had no recorded actions'
}

& $Exe uninstall $journal --json --headless --automation
if ($LASTEXITCODE -ne 0) {
    throw "bundled-exec uninstall failed: exit $LASTEXITCODE"
}
