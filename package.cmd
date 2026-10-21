@echo off
chcp 65001 >nul
setlocal EnableExtensions EnableDelayedExpansion
rem ---------------------------------------------------------------------------
rem 打包 release 版本：NSIS 安装器 + 便携版 zip + SHA-256。
rem 输出: artifacts\v<版本>\
rem
rem 用法:
rem   package.cmd                  构建并打包
rem   package.cmd --skip-build     复用 target\release\tug-desktop.exe，只重新打包
rem   package.cmd --no-installer   跳过安装器（已有安装器保留）
rem ---------------------------------------------------------------------------

call "%~dp0scripts\_env.cmd"
if errorlevel 1 exit /b 1
cd /d "%TUG_ROOT%"

set "SKIP_BUILD=0"
set "NO_INSTALLER=0"

:parse_args
if "%~1"=="" goto :args_done
if /i "%~1"=="--skip-build" (
  set "SKIP_BUILD=1"
  shift
  goto :parse_args
)
if /i "%~1"=="--no-installer" (
  set "NO_INSTALLER=1"
  shift
  goto :parse_args
)
if /i "%~1"=="help" goto :usage
if /i "%~1"=="-h" goto :usage
echo [错误] 未知参数: %~1
goto :usage_fail

:args_done
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
if not defined TUG_VERSION (
  echo [错误] 无法从 Cargo.toml 读取版本号。
  exit /b 1
)

if "%SKIP_BUILD%"=="1" goto :build_done
echo [1/4] 构建 release 可执行文件...
pushd "%TUG_DESKTOP%"
call %TUG_NPM% run tauri -- build --no-bundle
set "RC=!ERRORLEVEL!"
popd
if not "!RC!"=="0" (
  echo [错误] 构建失败，退出码 !RC!。
  exit /b !RC!
)
goto :build_done

:build_done
set "EXE=%TUG_TARGET%\release\tug-desktop.exe"
if not exist "%EXE%" (
  echo [错误] 未找到 %EXE%
  echo        请先运行 package.cmd（不要加 --skip-build）完成构建。
  exit /b 1
)

set "OUT_DIR=%TUG_ARTIFACTS%\v%TUG_VERSION%"
set "SETUP_NAME=Tug-%TUG_VERSION%-x64-setup.exe"
set "PORTABLE_NAME=Tug-%TUG_VERSION%-portable"
set "ZIP_NAME=%PORTABLE_NAME%.zip"
set "PORTABLE_DIR=%OUT_DIR%\%PORTABLE_NAME%"

if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
if not exist "%OUT_DIR%" (
  echo [错误] 无法创建输出目录: %OUT_DIR%
  exit /b 1
)
if exist "%PORTABLE_DIR%" rmdir /s /q "%PORTABLE_DIR%"
mkdir "%PORTABLE_DIR%"
copy /y "%EXE%" "%PORTABLE_DIR%\Tug.exe" >nul
if errorlevel 1 (
  echo [错误] 复制可执行文件失败。
  exit /b 1
)

rem Portable README (UTF-8 via the active code page).
> "%PORTABLE_DIR%\README.txt" echo Tug %TUG_VERSION% 便携版
>> "%PORTABLE_DIR%\README.txt" echo ================
>> "%PORTABLE_DIR%\README.txt" echo.
>> "%PORTABLE_DIR%\README.txt" echo 解压后双击 Tug.exe 运行，无需安装。
>> "%PORTABLE_DIR%\README.txt" echo.
>> "%PORTABLE_DIR%\README.txt" echo 要求:
>> "%PORTABLE_DIR%\README.txt" echo - Windows 10/11 x64
>> "%PORTABLE_DIR%\README.txt" echo - WebView2 Runtime（Windows 11 自带；Windows 10 如缺失请安装）
>> "%PORTABLE_DIR%\README.txt" echo.
>> "%PORTABLE_DIR%\README.txt" echo 用户数据:
>> "%PORTABLE_DIR%\README.txt" echo - %%APPDATA%%\Tug        设置与日志
>> "%PORTABLE_DIR%\README.txt" echo - %%LOCALAPPDATA%%\Tug   任务库与 BT session
>> "%PORTABLE_DIR%\README.txt" echo.
>> "%PORTABLE_DIR%\README.txt" echo 卸载: 直接删除本目录。

echo [2/4] 生成 NSIS 安装器...
if "%NO_INSTALLER%"=="1" (
  echo       跳过（--no-installer）
  goto :installer_done
)
if not defined TUG_MAKENSIS (
  echo       [警告] 未找到 makensis.exe，跳过安装器。
  echo              可安装 NSIS，或先运行一次 npm run tauri -- build 让 Tauri 下载 NSIS。
  goto :installer_done
)
"%TUG_MAKENSIS%" /INPUTCHARSET UTF8 ^
  /DAPP_VERSION=%TUG_VERSION% ^
  /DAPP_EXE="%EXE%" ^
  /DOUT_FILE="%OUT_DIR%\%SETUP_NAME%" ^
  /DAPP_ICON="%TUG_ICON%" ^
  "%TUG_ROOT%\packaging\nsis\installer.nsi"
set "RC=!ERRORLEVEL!"
if not "!RC!"=="0" (
  echo [错误] NSIS 打包失败，退出码 !RC!。
  exit /b !RC!
)
if not exist "%OUT_DIR%\%SETUP_NAME%" (
  echo [错误] NSIS 未生成安装器: %OUT_DIR%\%SETUP_NAME%
  exit /b 1
)

:installer_done
echo [3/4] 生成便携版 zip...
set "ZIP_PATH=%OUT_DIR%\%ZIP_NAME%"
if exist "%ZIP_PATH%" del /q "%ZIP_PATH%"
where tar.exe >nul 2>nul
if errorlevel 1 (
  powershell -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -Path '%PORTABLE_DIR%\*' -DestinationPath '%ZIP_PATH%' -Force"
  set "RC=!ERRORLEVEL!"
) else (
  pushd "%PORTABLE_DIR%"
  tar -a -c -f "%ZIP_PATH%" .
  set "RC=!ERRORLEVEL!"
  popd
)
if not "!RC!"=="0" (
  echo [错误] 生成 zip 失败，退出码 !RC!。
  exit /b !RC!
)
if not exist "%ZIP_PATH%" (
  echo [错误] 未生成 zip: %ZIP_PATH%
  exit /b 1
)

echo [4/4] 计算 SHA-256...
powershell -NoProfile -ExecutionPolicy Bypass -File "%TUG_ROOT%\scripts\sha256.ps1" -Directory "%OUT_DIR%"
if errorlevel 1 (
  echo [错误] SHA-256 计算失败。
  exit /b 1
)

echo.
echo [完成] 产物目录: %OUT_DIR%
dir /b "%OUT_DIR%"
echo.
exit /b 0

:usage
echo 用法:
echo   package.cmd                  构建并打包（NSIS + portable zip）
echo   package.cmd --skip-build     复用 target\release\tug-desktop.exe 重新打包
echo   package.cmd --no-installer   跳过安装器（已有安装器保留）
echo   package.cmd help             显示帮助
echo.
echo 产物输出到 artifacts\v^<版本^>\。
exit /b 0

:usage_fail
echo 用法:
echo   package.cmd [--skip-build] [--no-installer] [help]
exit /b 2
