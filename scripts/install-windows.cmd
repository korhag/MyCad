@echo off
setlocal DisableDelayedExpansion
set "EntoCadLaunch="
if "%~1"=="" set "EntoCadLaunch=-Launch"
rem Double-click installer. The PowerShell entry point also accepts automation options.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-windows.ps1" %EntoCadLaunch% %*
if errorlevel 1 (
    echo Installation failed. See the error dialog or installer log for details.
    pause
    exit /b 1
)
exit /b 0
