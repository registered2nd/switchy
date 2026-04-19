; Switchy NSIS installer hooks.
;
; Wired via tauri.conf.json → bundle.windows.nsis.installerHooks.
; Tauri's generated installer.nsi inserts these macros at the relevant
; lifecycle points; see `!ifmacrodef NSIS_HOOK_*` guards in the template.
;
; Goals (BACKLOG #4):
;   1. Force-kill switchy.exe before UNINSTALL so file locks don't prevent
;      teardown. Install path is left alone so Tauri's built-in
;      CheckIfAppIsRunning prompt ("Switchy is running, close it?") still
;      fires — surprising the user with a silent kill on upgrade is worse.
;   2. After uninstall, sweep residue in $INSTDIR (webview cache, logs) so a
;      fresh install doesn't inherit stale state.

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping ${PRODUCTNAME} if running..."
  nsExec::Exec 'taskkill /F /IM "${MAINBINARYNAME}.exe" /T'
  Pop $0
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Best-effort recursive cleanup of install dir residue (webview cache,
  ; logs, etc.). The main section already deleted the tracked files; this
  ; catches everything the app wrote at runtime that NSIS doesn't track.
  RMDir /r "$INSTDIR\EBWebView"
  RMDir /r "$INSTDIR"
!macroend
