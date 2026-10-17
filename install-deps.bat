@echo off
setlocal EnableExtensions

rem Install the Windows tools needed to compile MediaBridge for x86_64-pc-windows-msvc.
where winget >nul 2>nul
if errorlevel 1 (
  echo Windows Package Manager ^(winget^) is required. Install or update "App Installer" from Microsoft Store, then rerun this file.
  exit /b 1
)

where rustc >nul 2>nul
if errorlevel 1 (
  echo Installing Rustup and the Rust compiler...
  winget install --id Rustlang.Rustup --exact --accept-source-agreements --accept-package-agreements
  if errorlevel 1 exit /b 1
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
)

where cargo >nul 2>nul
if errorlevel 1 (
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
)
where cargo >nul 2>nul
if errorlevel 1 (
  echo Cargo was not found after installing Rustup. Close and reopen Command Prompt, then rerun this file.
  exit /b 1
)

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VSINSTALL="
if exist "%VSWHERE%" (
  for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%I"
)

if not defined VSINSTALL (
  echo Installing Visual Studio Build Tools with the C++ workload...
  winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --accept-source-agreements --accept-package-agreements --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  if errorlevel 1 exit /b 1
  if exist "%VSWHERE%" (
    for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%I"
  )
)

if not defined VSINSTALL (
  echo Visual Studio Build Tools installation was not detected.
  echo Install the "Desktop development with C++" workload, then rerun this file.
  exit /b 1
)

set "VCVARS=%VSINSTALL%\VC\Auxiliary\Build\vcvars64.bat"
if not exist "%VCVARS%" (
  echo Could not find the x64 Visual C++ environment script: "%VCVARS%"
  exit /b 1
)
call "%VCVARS%" >nul
if errorlevel 1 exit /b 1

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
rustup toolchain install stable-x86_64-pc-windows-msvc
if errorlevel 1 exit /b 1

cd /d "%~dp0"
echo Building MediaBridge for Windows...
cargo +stable-x86_64-pc-windows-msvc build --release
if errorlevel 1 (
  echo Build failed. The compiler output above contains the error details.
  exit /b 1
)
echo.
echo Build complete: "%~dp0target\release\mediabridge.exe"
endlocal
