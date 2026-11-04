@echo off
setlocal DisableDelayedExpansion
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\..\scripts\install-windows.ps1" -Uninstall %*
if errorlevel 1 (
    echo Uninstall failed. Close EntoCAD and check the installer log.
    pause
    exit /b 1
)
pause
exit /b 0
