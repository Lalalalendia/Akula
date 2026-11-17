; VIDEO-18b-16: isolated NON-RELEASE legacy-flat -> versioned-current NSIS bridge.
; This does NOT change product shortcuts, auto-update or publisher identity.
; Keep legacy flat five-file entrypoints available while current/ is staged.
!ifndef LAB_ID
  !error "Missing isolated lab ID"
!endif
!ifndef LAB_STAGE
  !error "Missing trusted lab staging"
!endif
!ifndef LAB_OUTPUT
  !error "Missing lab output"
!endif
Unicode true
Name "AkulaVideo18b16ProtectedVersionedNonreleaseLab"
OutFile "${LAB_OUTPUT}"
InstallDir "$PROGRAMFILES64\AkulaVideo18bRealNsisLab-${LAB_ID}"
RequestExecutionLevel admin
SilentInstall silent
SilentUnInstall silent
AutoCloseWindow true

Function .onInit
  StrCpy $INSTDIR "$PROGRAMFILES64\AkulaVideo18bRealNsisLab-${LAB_ID}"
FunctionEnd

Section "Stage exact five files in current while legacy root remains operational"
  ; Only an already installed, isolated five-EXE laboratory root is accepted.
  IfFileExists "$INSTDIR\akula-app.exe" +2 0
    Abort "VIDEO18B16_HOLD: legacy Akula executable missing"
  IfFileExists "$INSTDIR\shiva-pdf-preview.exe" +2 0
    Abort "VIDEO18B16_HOLD: legacy Shiva executable missing"
  IfFileExists "$INSTDIR\akula-video-mux.exe" +2 0
    Abort "VIDEO18B16_HOLD: legacy mux executable missing"
  IfFileExists "$INSTDIR\akula-yt-dlp.exe" +2 0
    Abort "VIDEO18B16_HOLD: legacy yt-dlp executable missing"
  IfFileExists "$INSTDIR\akula-quickjs.exe" +2 0
    Abort "VIDEO18B16_HOLD: legacy QuickJS executable missing"
  IfFileExists "$INSTDIR\current\*.*" 0 +2
    Abort "VIDEO18B16_HOLD: current generation must be new"

  ; This initial current generation is NOT activated by any shortcut or updater.
  ; Do not delete or rename legacy files before an external verified/ACKed cutover.
  SetOutPath "$INSTDIR\current"
  File "/oname=akula-app.exe" "${LAB_STAGE}\akula-app.exe"
  File "/oname=shiva-pdf-preview.exe" "${LAB_STAGE}\shiva-pdf-preview.exe"
  File "/oname=akula-video-mux.exe" "${LAB_STAGE}\akula-video-mux.exe"
  File "/oname=akula-yt-dlp.exe" "${LAB_STAGE}\akula-yt-dlp.exe"
  File "/oname=akula-quickjs.exe" "${LAB_STAGE}\akula-quickjs.exe"
  SetOutPath "$INSTDIR"
  ; Replace the lab-only old uninstaller with one that knows BOTH generations.
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; Explicit names only: never recursively remove an unknown user's directory.
  Delete "$INSTDIR\current\akula-app.exe"
  Delete "$INSTDIR\current\shiva-pdf-preview.exe"
  Delete "$INSTDIR\current\akula-video-mux.exe"
  Delete "$INSTDIR\current\akula-yt-dlp.exe"
  Delete "$INSTDIR\current\akula-quickjs.exe"
  RMDir "$INSTDIR\current"

  Delete "$INSTDIR\akula-app.exe"
  Delete "$INSTDIR\shiva-pdf-preview.exe"
  Delete "$INSTDIR\akula-video-mux.exe"
  Delete "$INSTDIR\akula-yt-dlp.exe"
  Delete "$INSTDIR\akula-quickjs.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
