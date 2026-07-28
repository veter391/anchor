; Anchor NSIS install hooks.
;
; The speech-runtime DLLs (sherpa-onnx + ONNX Runtime) and the bundled Visual
; C++ runtime must sit NEXT TO anchor.exe so it loads them at startup. Tauri
; bundles the `installer-libs` resource into $INSTDIR\installer-libs\; copy the
; DLLs up to the install root, drop the now-redundant folder, and remove the
; DLLs on uninstall.

!macro NSIS_HOOK_POSTINSTALL
  CopyFiles /SILENT "$INSTDIR\installer-libs\*.dll" "$INSTDIR"
  RMDir /r "$INSTDIR\installer-libs"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\*.dll"
!macroend
