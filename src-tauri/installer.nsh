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
  ; Mark this as an INSTALLED build. anchor.exe reads this marker (paths.rs) and
  ; then keeps its data (DB, transcripts, models) in a per-user folder OUTSIDE
  ; the install dir, so an upgrade/uninstall of $INSTDIR never wipes user data.
  FileOpen $0 "$INSTDIR\.installed" w
  FileWrite $0 "1"
  FileClose $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\*.dll"
!macroend
