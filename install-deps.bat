@echo off
setlocal EnableExtensions

rem Install the Windows tools needed to compile MediaBridge for x86_64-pc-windows-msvc.
rem Respect custom Cargo locations and resolve Rustup once before vcvars changes PATH.
if defined CARGO_HOME (
  set "PATH=%CARGO_HOME%\bin;%PATH%"
) else (
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
)
where rustup.exe >nul 2>nul
if errorlevel 1 (
  echo Installing Rustup and the Rust compiler...
  call :require_winget
  if errorlevel 1 exit /b 1
  winget install --id Rustlang.Rustup --exact --accept-source-agreements --accept-package-agreements
  if errorlevel 1 exit /b 1
)
set "RUSTUP_EXE="
for /f "delims=" %%I in ('where rustup.exe 2^>nul') do if not defined RUSTUP_EXE set "RUSTUP_EXE=%%I"
if not defined RUSTUP_EXE (
  echo Rustup was not found after installation. Reopen Command Prompt, then rerun this file.
  exit /b 1
)
"%RUSTUP_EXE%" --version >nul
if errorlevel 1 (
  echo Rustup could not run. Repair the Rust installation, then rerun this file.
  exit /b 1
)

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VSINSTALL="
if exist "%VSWHERE%" (
  for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%I"
)

if not defined VSINSTALL (
  echo Installing Visual Studio Build Tools with the C++ workload...
  call :require_winget
  if errorlevel 1 exit /b 1
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

"%RUSTUP_EXE%" run stable-x86_64-pc-windows-msvc rustc --version >nul 2>nul
if errorlevel 1 (
  "%RUSTUP_EXE%" toolchain install stable-x86_64-pc-windows-msvc
  if errorlevel 1 exit /b 1
)

cd /d "%~dp0"
echo Building MediaBridge for Windows...
if not exist "target" mkdir "target"
rc.exe /nologo /i "assets" /fo "target\mediabridge-icon.res" "assets\windows.rc"
if errorlevel 1 exit /b 1
"%RUSTUP_EXE%" run stable-x86_64-pc-windows-msvc cargo rustc --bin mediabridge --release --locked -- -C "link-arg=%~dp0target\mediabridge-icon.res"
if errorlevel 1 (
  echo Build failed. The compiler output above contains the error details.
  exit /b 1
)
echo.
echo Build complete: "%~dp0target\release\mediabridge.exe"
endlocal
exit /b 0

:require_winget
where winget >nul 2>nul
if errorlevel 1 (
  echo Windows Package Manager ^(winget^) is needed to install missing tools. Install App Installer from Microsoft Store, then rerun.
  exit /b 1
)
exit /b 0

