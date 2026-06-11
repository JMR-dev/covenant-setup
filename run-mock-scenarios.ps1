# run-mock-scenarios.ps1
# Script to run all default Covenant Setup UI mock scenarios sequentially (except install-slow).

$scenarios = @(
    "install-happy",
    "install-prompt",
    "install-fail-errata",
    "install-cancel-rollback",
    "uninstall-happy",
    "uninstall-reboot-prompt"
)

# 1. Ensure the application is built as self-contained for win-x64
Write-Host "Building Covenant.Setup.Ui (win-x64, self-contained)..." -ForegroundColor Cyan
dotnet build ui/Covenant.Setup.Ui/Covenant.Setup.Ui.csproj -r win-x64
if ($LASTEXITCODE -ne 0) {
    Write-Error "Build failed. Exiting."
    exit $LASTEXITCODE
}

# 2. Locate the executable
$exePath = "ui/Covenant.Setup.Ui/bin/x64/Debug/net10.0-windows10.0.19041.0/win-x64/Covenant.Setup.Ui.exe"
if (-not (Test-Path $exePath)) {
    $exePath = "ui/Covenant.Setup.Ui/bin/Debug/net10.0-windows10.0.19041.0/win-x64/Covenant.Setup.Ui.exe"
}
if (-not (Test-Path $exePath)) {
    Write-Error "Executable not found at: $exePath"
    exit 1
}

Write-Host "Starting mock scenarios sequence..." -ForegroundColor Cyan

foreach ($scenario in $scenarios) {
    Write-Host "`n========================================" -ForegroundColor Magenta
    Write-Host "Running scenario: $scenario" -ForegroundColor Yellow
    Write-Host "========================================" -ForegroundColor Magenta
    
    if ($scenario -eq "install-fail-errata") {
        Write-Host "Note: This scenario halts on the failure screen to let you inspect the UI (try the 'Copy' button). Click 'Close' or the Window X to proceed to the next scenario." -ForegroundColor Gray
    }

    if ($scenario -eq "install-cancel-rollback") {
        Write-Host "Note: Click 'Cancel' mid-progress to trigger the rollback flow, watch the revert steps, then click 'Close'." -ForegroundColor Gray
    }
    
    # Run and wait for exit
    Start-Process -FilePath $exePath -ArgumentList "--mock", $scenario -Wait -NoNewWindow
    
    Write-Host "Scenario $scenario completed/closed." -ForegroundColor Green
}

Write-Host "`nAll scenarios executed successfully!" -ForegroundColor Green
