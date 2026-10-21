@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem ---------------------------------------------------------------------------
rem 调用 tug CLI（无 GUI 下载 / BT 引擎验证）。
rem 示例:
rem   cli.cmd paths
rem   cli.cmd settings
rem   cli.cmd download "https://example.com/file.iso"
rem   cli.cmd bt "magnet:?xt=urn:btih:..." --seconds 60
rem ---------------------------------------------------------------------------

call "%~dp0scripts\_env.cmd"
if errorlevel 1 exit /b 1
cd /d "%TUG_ROOT%"

where cargo.exe >nul 2>nul
if errorlevel 1 (
  echo [错误] 未找到 cargo，请安装 Rust 工具链。
  exit /b 1
)

if "%~1"=="" (
  echo [提示] 未提供参数，显示 CLI 帮助。
  cargo run -p tug-cli -- --help
) else (
  cargo run -p tug-cli -- %*
)
exit /b %ERRORLEVEL%
