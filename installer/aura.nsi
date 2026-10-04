; Aura / EnvBox native installer (NSIS) — Kite-style per-machine install
Unicode True
!ifndef VERSION
  !error "VERSION is required; use scripts/build-installer.ps1"
!endif
Name "Aura"
OutFile "..\artifacts\aura-setup.exe"
InstallDir "$PROGRAMFILES64\Aura"
; 独立保存安装目录，卸载时保留，便于下一次安装继续使用用户选择的盘符。
; 覆盖安装依赖这一项：默认目录与上一版一致，新文件就地覆盖旧文件。
InstallDirRegKey HKLM "Software\Aura" "InstallLocation"
RequestExecutionLevel admin

!include "MUI2.nsh"
!include "LogicLib.nsh"
!define MUI_ABORTWARNING
!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; 用户确认开始安装后才关闭旧 Aura。取消安装向导时，旧版继续运行。
Section "-关闭旧版 Aura" SEC_CLOSE_OLD
  SectionIn RO
  ; 只结束 Aura 自身进程，不递归结束它唤起的目标应用（taskkill 不带 /T）。
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM envbox-app.exe'
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM envbox-broker.exe'
  Sleep 300
SectionEnd

; 覆盖安装不单独卸载旧版本：InstallDirRegKey 已把默认目录对齐到上一版安装位置，
; 下面的主程序 section 直接就地覆盖文件、快捷方式和卸载器。
; 用户配置在 %LOCALAPPDATA%\com.aura.envbox，安装器不写入、卸载也不删除。
Section "Aura 主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\artifacts\envbox-app.exe"
  File "..\artifacts\envbox.exe"
  File "..\artifacts\envbox-broker.exe"
  File "..\artifacts\envbox-supervisor.exe"
  File "..\artifacts\envbox-probe.exe"
  File "..\artifacts\envbox-browser-probe.exe"
  File "..\artifacts\envbox-suspended-helper.exe"
  File "..\artifacts\envbox-runtime64.dll"
  File "..\artifacts\envbox-runtime32.dll"
  File "..\icons\icon.ico"
  File "..\LICENSE"
  File "..\THIRD_PARTY_NOTICES.txt"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Aura" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "DisplayName" "Aura"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "Publisher" "EnvBox"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura" "DisplayIcon" "$INSTDIR\envbox-app.exe"
SectionEnd

Section "开始菜单快捷方式" SEC_START
  CreateDirectory "$SMPROGRAMS\Aura"
  CreateShortCut "$SMPROGRAMS\Aura\Aura.lnk" "$INSTDIR\envbox-app.exe" "" "$INSTDIR\envbox-app.exe" 0 SW_SHOWNORMAL "" "Aura"
  CreateShortCut "$SMPROGRAMS\Aura\EnvBox CLI.lnk" "$INSTDIR\envbox.exe" "" "$INSTDIR\envbox.exe" 0 SW_SHOWNORMAL "" "EnvBox CLI"
  CreateShortCut "$SMPROGRAMS\Aura\Environment Probe.lnk" "$INSTDIR\envbox-probe.exe" "" "$INSTDIR\envbox-probe.exe" 0 SW_SHOWNORMAL "" "Environment Probe"
SectionEnd

Section "桌面快捷方式" SEC_DESKTOP
  CreateShortCut "$DESKTOP\Aura.lnk" "$INSTDIR\envbox-app.exe" "" "$INSTDIR\envbox-app.exe" 0 SW_SHOWNORMAL "" "Aura"
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\Aura.lnk"
  Delete "$SMPROGRAMS\Aura\Aura.lnk"
  Delete "$SMPROGRAMS\Aura\EnvBox CLI.lnk"
  Delete "$SMPROGRAMS\Aura\Environment Probe.lnk"
  RMDir "$SMPROGRAMS\Aura"
  Delete "$INSTDIR\envbox-app.exe"
  Delete "$INSTDIR\envbox.exe"
  Delete "$INSTDIR\envbox-broker.exe"
  Delete "$INSTDIR\envbox-supervisor.exe"
  Delete "$INSTDIR\envbox-probe.exe"
  Delete "$INSTDIR\envbox-browser-probe.exe"
  Delete "$INSTDIR\envbox-suspended-helper.exe"
  Delete "$INSTDIR\envbox-runtime64.dll"
  Delete "$INSTDIR\envbox-runtime32.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Aura"
  ; Software\Aura\InstallLocation 保留，供下次安装对齐目录。
SectionEnd
