$logDir = Join-Path $PWD "logs"
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
"post-install script ran at $(Get-Date -Format o)" | Set-Content -Path (Join-Path $logDir "post_install.txt")
