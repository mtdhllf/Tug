@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem ---------------------------------------------------------------------------
rem 开发模式：启动 Vite + Tauri（热更新），Ctrl+C 退出。
rem 用法: dev.cmd
rem ---------------------------------------------------------------------------

call "%~dp0scripts\_env.cmd"
if errorlevel 1 exit /b 1
cd /d "%TUG_ROOT%"

if /i "%~1"=="help" goto :usage
if /i "%~1"=="-h" goto :usage

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

echo [运行] 启动 Tauri 开发模式，按 Ctrl+C 退出。
pushd "%TUG_DESKTOP%"
call %TUG_NPM% run tauri -- dev %*
set "RC=%ERRORLEVEL%"
popd

if not "%RC%"=="0" echo [错误] 开发模式退出，代码 %RC%。
exit /b %RC%

:usage
echo 用法:
echo   dev.cmd          启动开发模式（Ctrl+C 退出）
echo   dev.cmd help     显示帮助
exit /b 0
