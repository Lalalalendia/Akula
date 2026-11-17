; VIDEO-18b-03: genuine non-release NSIS with five inert mock files only.
!ifndef LAB_ID
  !error "Missing isolated lab ID"
!endif
!ifndef LAB_STAGE
  !error "Missing lab staging"
!endif
!ifndef LAB_OUTPUT
  !error "Missing lab output"
!endif
Unicode true
Name "AkulaVideo18bIsolatedNonreleaseLab"
OutFile "${LAB_OUTPUT}"
InstallDir "$PROGRAMFILES64\AkulaVideo18bNsisLab-${LAB_ID}"
RequestExecutionLevel admin
SilentInstall silent
SilentUnInstall silent
AutoCloseWindow true
Function .onInit
  StrCpy $INSTDIR "$PROGRAMFILES64\AkulaVideo18bNsisLab-${LAB_ID}"
FunctionEnd
Section "Install lab-only mock executables"
  SetOutPath "$INSTDIR"
  File "/oname=akula-app.exe" "${LAB_STAGE}\akula-app.exe"
  File "/oname=shiva-pdf-preview.exe" "${LAB_STAGE}\shiva-pdf-preview.exe"
  File "/oname=akula-video-mux.exe" "${LAB_STAGE}\akula-video-mux.exe"
  File "/oname=akula-yt-dlp.exe" "${LAB_STAGE}\akula-yt-dlp.exe"
  File "/oname=akula-quickjs.exe" "${LAB_STAGE}\akula-quickjs.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd
Section "Uninstall"
  Delete "$INSTDIR\akula-app.exe"
  Delete "$INSTDIR\shiva-pdf-preview.exe"
  Delete "$INSTDIR\akula-video-mux.exe"
  Delete "$INSTDIR\akula-yt-dlp.exe"
  Delete "$INSTDIR\akula-quickjs.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
