[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$ResultPath,
    [int]$TimeoutSeconds = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$installer = $null
$startedAt = Get-Date

try {
    if (-not (Test-Path -LiteralPath $InstallerPath)) {
        throw "Installer not found: $InstallerPath"
    }

    $resultDir = Split-Path -Parent $ResultPath
    if ($resultDir) {
        New-Item -ItemType Directory -Force -Path $resultDir | Out-Null
    }
    Remove-Item -LiteralPath $ResultPath -Force -ErrorAction SilentlyContinue

    $installer = Start-Process -FilePath $InstallerPath -ArgumentList @("--headed", "--automation") -PassThru

    if (-not $installer.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $installer.Id -Force -ErrorAction SilentlyContinue
        throw "Installer timed out after $TimeoutSeconds seconds."
    }

    $result = [ordered]@{
        success      = ($installer.ExitCode -eq 0)
        exitCode     = [int]$installer.ExitCode
        installerPath = $InstallerPath
        startedAt    = $startedAt.ToString("o")
        finishedAt   = (Get-Date).ToString("o")
    }
    $result | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8

    if ($installer.ExitCode -ne 0) {
        exit $installer.ExitCode
    }
}
catch {
    $failure = [ordered]@{
        success    = $false
        error      = $_.Exception.Message
        startedAt  = $startedAt.ToString("o")
        finishedAt = (Get-Date).ToString("o")
    }
    $failure | ConvertTo-Json | Set-Content -LiteralPath $ResultPath -Encoding UTF8
    exit 1
}
