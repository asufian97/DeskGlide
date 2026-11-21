$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

Write-Host "=== LanBridge Windows Host Setup (Optional Secondary Backend) ===" -ForegroundColor Cyan

$py = Get-Command python -ErrorAction SilentlyContinue
if (-not $py -or $py.Source -like "*WindowsApps*") {
    Write-Host "Real Python 3.10+ not found on PATH. Install Python 3.12 first:" -ForegroundColor Yellow
    Write-Host "  winget install --id Python.Python.3.12 -e"
    exit 1
}

if (-not (Test-Path ".venv")) {
    & $py.Source -m venv .venv
}

& ".\.venv\Scripts\pip.exe" install --upgrade pip
& ".\.venv\Scripts\pip.exe" install -e "."

$addFw = Read-Host "Allow TCP port 8443 in Windows Defender Firewall for LanBridge? (y/N)"
if ($addFw -match "^[yY]$") {
    netsh advfirewall firewall delete rule name="LanBridge (HTTPS/WSS)" 2>$null | Out-Null
    netsh advfirewall firewall add rule name="LanBridge (HTTPS/WSS)" dir=in action=allow protocol=TCP localport=8443 | Out-Null
    Write-Host "Firewall rule added for TCP 8443." -ForegroundColor Green
}

Write-Host "`nRunning LanBridge System Doctor..." -ForegroundColor Cyan
& ".\.venv\Scripts\python.exe" doctor.py

Write-Host "`nNote: Windows Host mode requires GStreamer 1.x MSVC 64-bit runtime + PyGObject (gi)." -ForegroundColor Yellow
Write-Host "For controlling Zorin OS FROM Windows, Windows needs NO install at all — just open Chrome/Edge!" -ForegroundColor Green
