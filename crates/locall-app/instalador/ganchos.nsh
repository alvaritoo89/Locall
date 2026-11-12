; Locall's own steps in the installer and uninstaller (ADR 0032). Tauri's template inserts these macros at fixed points.
;
; - Before installing (also when updating) and before uninstalling: close Locall's core if it is running
;   (`locall stop`), so its files can be replaced or removed. The app window is handled by Tauri's template.
; - After installing: add the install folder to the user's PATH, so `locall` works in any new terminal.
; - Before uninstalling: remove it from the PATH.
;
; The PATH is changed by locall.exe itself (in Rust), not here: NSIS has a limit on text length and could cut a long
; PATH. Nothing here touches the user's data in %LOCALAPPDATA%\Locall: uninstalling Locall keeps models, conversations
; and settings.
;
; nsExec runs the command without a console window; its result is popped and ignored on purpose: a failure here (for
; example, Locall was not running) must not stop the installation.

!macro NSIS_HOOK_PREINSTALL
  ${If} ${FileExists} "$INSTDIR\locall.exe"
    nsExec::Exec '"$INSTDIR\locall.exe" stop'
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\locall.exe" --instalador-anadir-al-path'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\locall.exe"
    nsExec::Exec '"$INSTDIR\locall.exe" stop'
    Pop $0
    nsExec::Exec '"$INSTDIR\locall.exe" --instalador-quitar-del-path'
    Pop $0
  ${EndIf}
!macroend
