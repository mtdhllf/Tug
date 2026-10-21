@echo off
chcp 65001 >nul
setlocal EnableExtensions
rem ---------------------------------------------------------------------------
rem 本地检查 / 测试。
rem 用法:
rem   test.cmd             全部：前端 check + build、fmt --check、clippy、Rust 测试
rem   test.cmd quick       只跑 Rust 测试（最快）
rem   test.cmd rust        fmt --check + clippy + Rust 测试
rem   test.cmd frontend    前端类型检查 + 构建
rem   test.cmd fmt         自动执行 cargo fmt --all（修改文件）
rem ---------------------------------------------------------------------------

call "%~dp0scripts\_env.cmd"
if errorlevel 1 exit /b 1
cd /d "%TUG_ROOT%"

set "MODE=%~1"
if not defined MODE set "MODE=all"
if /i "%MODE%"=="help" goto :usage
if /i "%MODE%"=="-h" goto :usage
if /i "%MODE%"=="all" goto :mode_all
if /i "%MODE%"=="rust" goto :mode_rust
if /i "%MODE%"=="quick" goto :mode_quick
if /i "%MODE%"=="frontend" goto :mode_frontend
if /i "%MODE%"=="fmt" goto :mode_fmt

echo [错误] 未知模式: %MODE%
goto :usage_fail

:mode_all
set "FAILED=0"
call :check_frontend
if errorlevel 1 set "FAILED=1"
call :build_frontend
if errorlevel 1 set "FAILED=1"
call :check_fmt
if errorlevel 1 set "FAILED=1"
call :check_clippy
if errorlevel 1 set "FAILED=1"
call :run_tests
if errorlevel 1 set "FAILED=1"
goto :summary

:mode_rust
set "FAILED=0"
call :check_fmt
if errorlevel 1 set "FAILED=1"
call :check_clippy
if errorlevel 1 set "FAILED=1"
call :run_tests
if errorlevel 1 set "FAILED=1"
goto :summary

:mode_quick
set "FAILED=0"
call :run_tests
if errorlevel 1 set "FAILED=1"
goto :summary

:mode_frontend
set "FAILED=0"
call :check_frontend
if errorlevel 1 set "FAILED=1"
call :build_frontend
if errorlevel 1 set "FAILED=1"
goto :summary

:mode_fmt
echo [格式化] cargo fmt --all
cargo fmt --all
if errorlevel 1 (
  echo [失败] cargo fmt 执行失败。
  exit /b 1
)
echo [完成] 已格式化 Rust 代码。
exit /b 0

:summary
echo.
if "%FAILED%"=="0" (
  echo [通过] 所有检查通过。
  exit /b 0
)
echo [失败] 存在未通过的检查，请查看上方输出。
exit /b 1

rem ---------------------------------------------------------------------------
rem Steps. Each returns 0 on success, non-zero on failure.
rem ---------------------------------------------------------------------------

:check_frontend
echo.
echo [检查] 前端 TypeScript: npm run check
where node.exe >nul 2>nul
if errorlevel 1 (
  echo [错误] 未找到 Node.js。
  exit /b 1
)
pushd "%TUG_DESKTOP%"
call %TUG_NPM% run check
set "RC=%ERRORLEVEL%"
popd
exit /b %RC%

:build_frontend
echo.
echo [检查] 前端构建: npm run build
pushd "%TUG_DESKTOP%"
call %TUG_NPM% run build
set "RC=%ERRORLEVEL%"
popd
exit /b %RC%

:check_fmt
echo.
echo [检查] Rust 格式: cargo fmt --all -- --check
cargo fmt --all -- --check
exit /b %ERRORLEVEL%

:check_clippy
echo.
echo [检查] Clippy: cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
exit /b %ERRORLEVEL%

:run_tests
echo.
echo [检查] Rust 测试: cargo test -p tug-core -p tug-http -p tug-torrent -p tug-service
cargo test -p tug-core -p tug-http -p tug-torrent -p tug-service
exit /b %ERRORLEVEL%

:usage
echo 用法:
echo   test.cmd             全部检查（前端 check + build、fmt --check、clippy、测试）
echo   test.cmd quick       只跑 Rust 测试
echo   test.cmd rust        fmt --check + clippy + Rust 测试
echo   test.cmd frontend    前端类型检查 + 构建
echo   test.cmd fmt         自动格式化 Rust 代码
echo   test.cmd help        显示帮助
exit /b 0

:usage_fail
echo 用法:
echo   test.cmd [all^|quick^|rust^|frontend^|fmt^|help]
exit /b 2
