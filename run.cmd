@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem ---------------------------------------------------------------------------
rem 构建并运行桌面应用（不生成安装包）。
rem 用法:
rem   run.cmd           release 构建后启动（较慢，但接近发布版）
rem   run.cmd debug     debug 构建后启动（编译更快，日志更全）
rem ---------------------------------------------------------------------------

call "%~dp0scripts\_env.cmd"
if errorlevel 1 exit /b 1
cd /d "%TUG_ROOT%"

if /i "%~1"=="help" goto :usage
if /i "%~1"=="-h" goto :usage
if /i "%~1"=="debug" (
  set "PROFILE=debug"
  set "TAURI_ARGS=--debug --no-bundle"
) else (
  if not "%~1"=="" (
    echo [错误] 未知参数: %~1
    goto :usage_fail
  )
  set "PROFILE=release"
  set "TAURI_ARGS=--no-bundle"
)

where node.exe >nul 2>nul
if errorlevel 1 (
  echo [错误] 未找到 Node.js，请安装 Node.js 22+ 并确保 node 在 PATH 中。
  exit /b 1
)
where cargo.exe >nul 2>nul
if errorlevel 1 (
  echo [错误] 未找到 cargo，请安装 Rust 工具链。
  exit /b 1
)

if not exist "%TUG_DESKTOP%\node_modules" (
  echo [准备] 首次运行，安装前端依赖...
  pushd "%TUG_DESKTOP%"
  call %TUG_NPM% install
  if errorlevel 1 (
    popd
    echo [错误] 前端依赖安装失败。
    exit /b 1
  )
  popd
)

echo [1/2] 构建 %PROFILE% 版本（不生成安装包）...
pushd "%TUG_DESKTOP%"
call %TUG_NPM% run tauri -- build %TAURI_ARGS%
set "RC=%ERRORLEVEL%"
popd
if not "%RC%"=="0" (
  echo [错误] 构建失败，退出码 %RC%。
  exit /b %RC%
)

if /i "%PROFILE%"=="debug" (
  set "EXE=%TUG_TARGET%\debug\tug-desktop.exe"
) else (
  set "EXE=%TUG_TARGET%\release\tug-desktop.exe"
)
if not exist "%EXE%" (
  echo [错误] 未找到构建产物: %EXE%
  exit /b 1
)

echo [2/2] 启动 %EXE%
"%EXE%"
set "RC=%ERRORLEVEL%"
if not "%RC%"=="0" echo [提示] 应用退出，代码 %RC%。
exit /b %RC%

:usage
echo 用法:
echo   run.cmd          构建 release 并启动
echo   run.cmd debug    构建 debug 并启动（更快）
echo   run.cmd help     显示帮助
exit /b 0

:usage_fail
echo 用法:
echo   run.cmd          构建 release 并启动
echo   run.cmd debug    构建 debug 并启动（更快）
exit /b 2
