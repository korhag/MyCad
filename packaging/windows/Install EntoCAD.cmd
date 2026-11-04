@echo off
setlocal DisableDelayedExpansion
set "EntoCadLaunch="
if "%~1"=="" set "EntoCadLaunch=-Launch"
rem Legacy entry point retained for existing installed update records.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\..\scripts\install-windows.ps1" %EntoCadLaunch% %*
if errorlevel 1 (
    echo Installation failed. See the error dialog or installer log for details.
    pause
    exit /b 1
)
exit /b 0
