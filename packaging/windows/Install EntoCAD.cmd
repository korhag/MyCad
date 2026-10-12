@echo off
echo Building EntoCAD. The first build can take several minutes.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
if errorlevel 1 (
    echo Install failed.
    pause
    exit /b 1
)
start "" "%LOCALAPPDATA%\Programs\EntoCAD\EntoCAD.exe"
