; Nabra installer hooks (Tauri NSIS). Included by tauri.release.conf.json → bundle.windows.nsis.installerHooks.
;
; After install: ask whether Nabra should start with Windows (default Yes; silent installs use Yes).
; The desktop shortcut is the "Create desktop shortcut" checkbox on the finish page (ticked by default).
; Both can be changed later: Nabra → Settings → General → Start with Windows.

!define NABRA_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

!macro NSIS_HOOK_POSTINSTALL
  ; Automatic updates (/UPDATE) keep the user's existing choice: never ask again.
  ${If} $UpdateMode = 1
    Goto nabra_startup_done
  ${EndIf}
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON1 \
    "Start Nabra automatically when you sign in to Windows?$\r$\n$\r$\nRecommended, so your dictation shortcut always works. You can change this later in Nabra → Settings." \
    /SD IDYES IDNO nabra_no_startup
    WriteRegStr HKCU "${NABRA_RUN_KEY}" "Nabra" '"$INSTDIR\${MAINBINARYNAME}.exe"'
    Goto nabra_startup_done
  nabra_no_startup:
    DeleteRegValue HKCU "${NABRA_RUN_KEY}" "Nabra"
  nabra_startup_done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "${NABRA_RUN_KEY}" "Nabra"
  ; "Delete app data" ticked: also remove the downloaded models (≈10 GB) and logs. Otherwise they're kept,
  ; so reinstalling or upgrading doesn't download them again.
  ${If} $DeleteAppDataCheckboxState = 1
    RMDir /r "$LOCALAPPDATA\Nabra\assets"
    RMDir /r "$LOCALAPPDATA\Nabra\logs"
    RMDir "$LOCALAPPDATA\Nabra"
  ${EndIf}
!macroend
