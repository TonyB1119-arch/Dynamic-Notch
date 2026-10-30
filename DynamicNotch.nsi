; Dynamic Notch installer (NSIS). Build on Linux or Windows:  makensis DynamicNotch.nsi
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "3.0.0"
!endif
!ifndef EXE
  !define EXE "target/x86_64-pc-windows-gnu/release/dynamic-notch.exe"
!endif

Name "Dynamic Notch"
OutFile "DynamicNotch-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\DynamicNotch"
InstallDirRegKey HKCU "Software\DynamicNotch" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "Dynamic Notch ${VERSION}"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\dynamic-notch.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Dynamic Notch"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Dynamic Notch"
  ; ask a running copy to quit so the file can be replaced
  IfFileExists "$INSTDIR\dynamic-notch.exe" 0 fresh
  ExecWait '"$INSTDIR\dynamic-notch.exe" quit'
  Sleep 1200
  fresh:

  SetOutPath "$INSTDIR"
  File "${EXE}"
  File "LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortCut "$SMPROGRAMS\Dynamic Notch.lnk" "$INSTDIR\dynamic-notch.exe"

  WriteRegStr HKCU "Software\DynamicNotch" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "DisplayName" "Dynamic Notch"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "Publisher" "Dynamic Notch"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "DisplayIcon" "$INSTDIR\dynamic-notch.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch" "NoRepair" 1
SectionEnd

; a silent install (an update) starts the app again by itself
Function .onInstSuccess
  IfSilent 0 done
  Exec '"$INSTDIR\dynamic-notch.exe"'
  done:
FunctionEnd

Section "Uninstall"
  IfFileExists "$INSTDIR\dynamic-notch.exe" 0 gone
  ExecWait '"$INSTDIR\dynamic-notch.exe" quit'
  Sleep 1200
  gone:

  Delete "$INSTDIR\dynamic-notch.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Dynamic Notch.lnk"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DynamicNotch"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\DynamicNotch"
  DeleteRegKey HKCU "Software\DynamicNotch"
SectionEnd
