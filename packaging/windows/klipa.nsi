; NSIS installer for klipa (Windows).
; Build:  makensis -DVERSION=0.1.0 packaging/windows/klipa.nsi
; Expects: dist\klipa.exe  and  packaging\icons\klipa.ico
; Output:  dist\klipa-<VERSION>-windows-x64-setup.exe

!ifndef VERSION
  !define VERSION "0.1.0"
!endif

!define APPNAME   "klipa"
!define COMPANY   "Petros Dhespollari"
!define WEBSITE   "https://klipa.peterdsp.dev"

Unicode true
SetCompressor /SOLID lzma
Name "${APPNAME}"
OutFile "..\..\dist\klipa-${VERSION}-windows-x64-setup.exe"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel admin
Icon "..\icons\klipa.ico"
UninstallIcon "..\icons\klipa.ico"

!include "MUI2.nsh"
!define MUI_ICON   "..\icons\klipa.ico"
!define MUI_UNICON "..\icons\klipa.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\klipa.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName"   "${APPNAME}"
VIAddVersionKey "CompanyName"   "${COMPANY}"
VIAddVersionKey "FileVersion"   "${VERSION}"
VIAddVersionKey "LegalCopyright" "(C) 2026 ${COMPANY} - MIT"

Section "klipa (required)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "..\..\dist\klipa.exe"
  File "..\icons\klipa.ico"

  ; Start Menu shortcut
  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut  "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\klipa.exe" "" "$INSTDIR\klipa.ico"

  ; Launch at login (klipa is a tray app)
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${APPNAME}" '"$INSTDIR\klipa.exe"'

  ; Add/Remove Programs entry
  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "Publisher" "${COMPANY}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "URLInfoAbout" "${WEBSITE}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "DisplayIcon" "$INSTDIR\klipa.ico"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\klipa.exe"
  Delete "$INSTDIR\klipa.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  RMDir  "$SMPROGRAMS\${APPNAME}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${APPNAME}"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd
