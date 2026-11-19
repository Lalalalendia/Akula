; VIDEO-TOOLS-01j: SIX-file, ephemeral-Minisign test installer.
; No publisher signature, no release, no stock Akula updater integration.
!ifndef LAB_STAGE
  !error "VIDEO_TOOLS_01J_HOLD: signed stage missing"
!endif
!ifndef LAB_OUTPUT
  !error "VIDEO_TOOLS_01J_HOLD: temporary installer output missing"
!endif
!include "LogicLib.nsh"
!include "WinVer.nsh"
Unicode true
Name "Akula Video Tools 01j NONRELEASE laboratory"
OutFile "${LAB_OUTPUT}"
InstallDir "$PROGRAMFILES64\Akula Video Tools"
RequestExecutionLevel admin
SilentInstall silent
SilentUnInstall silent
AutoCloseWindow true
ShowInstDetails nevershow
ShowUninstDetails nevershow

Function .onInit
  ${IfNot} ${AtLeastWin10}
    Abort "VIDEO_TOOLS_01J_HOLD: Windows 10 or later required"
  ${EndIf}
  StrCpy $INSTDIR "$PROGRAMFILES64\Akula Video Tools"
  IfFileExists "$INSTDIR\*.*" 0 +2
    Abort "VIDEO_TOOLS_01J_HOLD: protected Video Tools root already exists"
FunctionEnd

Section "Install only separate nonrelease Video Tools"
  SetOutPath "$INSTDIR"
  File "/oname=akula-video-mux.exe" "${LAB_STAGE}\akula-video-mux.exe"
  File "/oname=akula-yt-dlp.exe" "${LAB_STAGE}\akula-yt-dlp.exe"
  File "/oname=akula-quickjs.exe" "${LAB_STAGE}\akula-quickjs.exe"
  File "/oname=manifest.json" "${LAB_STAGE}\manifest.json"
  File "/oname=manifest.minisig" "${LAB_STAGE}\manifest.minisig"
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; Exact six files. Never Delete /r or touch user notes.
  Delete "$INSTDIR\akula-video-mux.exe"
  Delete "$INSTDIR\akula-yt-dlp.exe"
  Delete "$INSTDIR\akula-quickjs.exe"
  Delete "$INSTDIR\manifest.json"
  Delete "$INSTDIR\manifest.minisig"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
