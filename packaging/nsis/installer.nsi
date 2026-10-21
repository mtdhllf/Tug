; Tug per-user NSIS installer.
; Build:
;   makensis /DAPP_VERSION=0.5.0 /DAPP_EXE=...\tug-desktop.exe /DOUT_FILE=...\Tug-0.5.0-x64-setup.exe installer.nsi
; Optional: /DAPP_ICON=...\icon.ico
; APP_VERSION defaults to 0.5.0 so old manual commands keep working.

Unicode true
!include "MUI2.nsh"

!ifndef APP_EXE
  !define APP_EXE "..\..\target\release\tug-desktop.exe"
!endif
!ifndef OUT_FILE
  !define OUT_FILE "Tug-0.5.0-x64-setup.exe"
!endif
!ifndef APP_ICON
  !define APP_ICON "..\..\apps\desktop\src-tauri\icons\icon.ico"
!endif
!ifndef APP_VERSION
  !define APP_VERSION "0.5.0"
!endif

!define APP_NAME "Tug"
!define APP_ID "com.tug.app"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Tug"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Tug"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 64

!define MUI_ABORTWARNING
!define MUI_ICON "${APP_ICON}"
!define MUI_UNICON "${APP_ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\Tug.exe"
!define MUI_FINISHPAGE_RUN_TEXT "启动 Tug"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Section "Tug" SEC_MAIN
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /oname=Tug.exe "${APP_EXE}"
  WriteUninstaller "$INSTDIR\Uninstall Tug.exe"

  CreateShortCut "$SMPROGRAMS\Tug.lnk" "$INSTDIR\Tug.exe" "" "$INSTDIR\Tug.exe" 0
  CreateShortCut "$DESKTOP\Tug.lnk" "$INSTDIR\Tug.exe" "" "$INSTDIR\Tug.exe" 0

  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Tug"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Tug.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall Tug.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Tug.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  ; 应用启动时还会写入 IconUri/DisplayName；这里先提供卸载时的兜底。
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${APP_ID}" "DisplayName" "Tug"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${APP_ID}" "IconUri" "$INSTDIR\Tug.exe"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${APP_ID}" "IconBackgroundColor" "0"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$INSTDIR\Tug.exe"
  Delete "$INSTDIR\Uninstall Tug.exe"
  Delete "$SMPROGRAMS\Tug.lnk"
  Delete "$DESKTOP\Tug.lnk"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${APP_ID}"
SectionEnd
