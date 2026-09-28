; Tauri's uninstaller offers "Delete the application data" and removes the
; folders named after the bundle identifier (dev.modlobby.app). modlobby
; keeps its own under its name, so the box reaches them here:
;
;   %APPDATA%\modlobby           settings, logs, caches, presets, drafts
;                                (settings::config_dir)
;   %LOCALAPPDATA%\modlobby\data  the engine's: engines, games, maps, replays,
;                                screenshots, keybinds, widgets
;                                (lobby_runtime::launch::own_data_dir)
;
; Never during an update, and only when the box was ticked. Passwords are in
; the OS keyring and stay there.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\modlobby"
    RmDir /r "$LOCALAPPDATA\modlobby\data"
    ; Empty by now unless something else lives there, and never forced.
    RmDir "$LOCALAPPDATA\modlobby"
  ${EndIf}
!macroend
