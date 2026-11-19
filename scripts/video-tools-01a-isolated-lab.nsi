; VIDEO-TOOLS-01a: NON-RELEASE 3-PE installer fixture.
; Separate product identity; never changes the stock Akula package/updater.
; Only accepts exact-head, ephemeral GitHub runner-created inputs.
!ifndef LAB_ID
  !error "VIDEO_TOOLS_01A_HOLD: LAB_ID missing"
!endif
!ifndef LAB_STAGE
  !error "VIDEO_TOOLS_01A_HOLD: LAB_STAGE missing"
!endif
!ifndef LAB_OUTPUT
  !error "VIDEO_TOOLS_01A_HOLD: LAB_OUTPUT missing"
!endif
!include "LogicLib.nsh"
!include "WinVer.nsh"
Unicode true
Name "Akula Video Tools 01a nonrelease laboratory"
OutFile "${LAB_OUTPUT}"
InstallDir "$PROGRAMFILES64\AkulaVideoToolsLab-${LAB_ID}"
RequestExecutionLevel admin
SilentInstall silent
SilentUnInstall silent
AutoCloseWindow true
ShowInstDetails nevershow
ShowUninstDetails nevershow

Function .onInit
  ${IfNot} ${AtLeastWin10}
    Abort "VIDEO_TOOLS_01A_HOLD: Windows 10 or later required"
  ${EndIf}
  StrCpy $INSTDIR "$PROGRAMFILES64\AkulaVideoToolsLab-${LAB_ID}"
FunctionEnd

Section "Install only three isolated media tools"
  ; No automatic updates, Akula GUI/PDF replacements, registry entries,
  ; shortcuts, uninstall registry identity or recursive directory removal.
  IfFileExists "$INSTDIR\*.*" 0 +2
    Abort "VIDEO_TOOLS_01A_HOLD: target exists"
  SetOutPath "$INSTDIR"
  File "/oname=akula-video-mux.exe" "${LAB_STAGE}\akula-video-mux.exe"
  File "/oname=akula-yt-dlp.exe" "${LAB_STAGE}\akula-yt-dlp.exe"
  File "/oname=akula-quickjs.exe" "${LAB_STAGE}\akula-quickjs.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; Explicit names only. Refuse to delete user notes or arbitrary files.
  Delete "$INSTDIR\akula-video-mux.exe"
  Delete "$INSTDIR\akula-yt-dlp.exe"
  Delete "$INSTDIR\akula-quickjs.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
