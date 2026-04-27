[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$InstallerProcessId,
    [string]$WindowTitle = "covenant-setup",
    [int]$PollMilliseconds = 500
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName Microsoft.VisualBasic
Add-Type -AssemblyName System.Windows.Forms

while ($true) {
    $process = Get-Process -Id $InstallerProcessId -ErrorAction SilentlyContinue
    if (-not $process) {
        break
    }

    if ([Microsoft.VisualBasic.Interaction]::AppActivate($WindowTitle)) {
        Start-Sleep -Milliseconds 200
        [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    }

    Start-Sleep -Milliseconds $PollMilliseconds
}
