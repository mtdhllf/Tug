@echo off
rem ---------------------------------------------------------------------------
rem Internal helper: resolve project paths and tool commands.
rem Usage: call "%~dp0scripts\_env.cmd" || exit /b 1
rem Sets: TUG_ROOT TUG_DESKTOP TUG_TAURI TUG_TARGET TUG_ARTIFACTS
rem       TUG_ICON TUG_VERSION TUG_NPM TUG_MAKENSIS
rem ---------------------------------------------------------------------------

for %%I in ("%~dp0..") do set "TUG_ROOT=%%~fI"
set "TUG_DESKTOP=%TUG_ROOT%\apps\desktop"
set "TUG_TAURI=%TUG_DESKTOP%\src-tauri"
set "TUG_TARGET=%TUG_ROOT%\target"
set "TUG_ARTIFACTS=%TUG_ROOT%\artifacts"
set "TUG_ICON=%TUG_TAURI%\icons\icon.ico"

rem Workspace version, e.g. version = "1.0.0" in Cargo.toml.
set "TUG_VERSION="
for /f "tokens=2 delims== " %%V in ('findstr /r /c:"^version = " "%TUG_ROOT%\Cargo.toml"') do if not defined TUG_VERSION set "TUG_VERSION=%%~V"

rem npm is bundled with Node.js.
set "TUG_NPM="
where npm.cmd >nul 2>nul && set "TUG_NPM=npm"
if not defined TUG_NPM (
  echo [ERROR] npm not found. Install Node.js and make sure npm is on PATH.
  exit /b 1
)

rem tauri-winres needs the Windows SDK resource compiler. Cargo invoked outside
rem the Tauri CLI may not have rc.exe on PATH, so point RC at the newest x64 SDK.
if defined RC goto :rc_ready
where rc.exe >nul 2>nul
if not errorlevel 1 goto :rc_ready
set "TUG_RC="
for /f "delims=" %%V in ('dir /b /ad /o-n "%ProgramFiles(x86)%\Windows Kits\10\bin\10.*" 2^>nul') do if not defined TUG_RC if exist "%ProgramFiles(x86)%\Windows Kits\10\bin\%%V\x64\rc.exe" set "TUG_RC=%ProgramFiles(x86)%\Windows Kits\10\bin\%%V\x64\rc.exe"
if not defined TUG_RC goto :rc_ready
set "RC=%TUG_RC%"

:rc_ready

rem Tauri downloads NSIS here on first bundle; also accept a standalone NSIS.
set "TUG_MAKENSIS="
if exist "%LOCALAPPDATA%\tauri\NSIS\Bin\makensis.exe" set "TUG_MAKENSIS=%LOCALAPPDATA%\tauri\NSIS\Bin\makensis.exe"
if not defined TUG_MAKENSIS if exist "%ProgramFiles(x86)%\NSIS\makensis.exe" set "TUG_MAKENSIS=%ProgramFiles(x86)%\NSIS\makensis.exe"
if not defined TUG_MAKENSIS if exist "%ProgramFiles%\NSIS\makensis.exe" set "TUG_MAKENSIS=%ProgramFiles%\NSIS\makensis.exe"
if not defined TUG_MAKENSIS for /f "delims=" %%M in ('where makensis.exe 2^>nul') do if not defined TUG_MAKENSIS set "TUG_MAKENSIS=%%M"

exit /b 0
