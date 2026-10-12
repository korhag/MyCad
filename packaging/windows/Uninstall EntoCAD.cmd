@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Uninstall
if errorlevel 1 (
    echo Uninstall failed.
    pause
    exit /b 1
)
pause
