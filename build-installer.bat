@echo off
setlocal
echo Rebuilding MediaBridge from the current source before creating the installer...
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-installer.ps1"
if errorlevel 1 (
  echo.
  echo Installer packaging failed. See the error above.
  exit /b 1
)
echo.
echo Installer is ready in "%~dp0dist\MediaBridge-Setup.exe".
endlocal
