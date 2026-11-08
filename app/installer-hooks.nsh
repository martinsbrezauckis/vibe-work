; Vibe Work NSIS uninstall data handlers. Included by the pinned Tauri 2.10.1 template.
; Called after standalone GUI removal; updates, /P and /S retain all user data.
; Program integration, including per-run video sandboxes, is removed at every standalone level.
; Invariant: tier 2 deletes only listed settings/preferences/caches and app-managed runtimes. Only the
; separate tier-3 choice deletes managed sessions, browser sign-ins and the private/legacy vault.
; Every app-owned ancestor is checked; links/reparse points are never traversed.
; "Send to Vibe Work": an interactive install asks once whether to add the Explorer right-click entries (the same keys
; and shortcut the app's Settings > General > Right-click menus writes, crates/vw-platform/src/send_to_windows.rs);
; the uninstall removes them, also when the switch removed them already. Updates keep them.
; Program files: install and uninstall also delete the install-folder files earlier versions shipped and this one
; does not (VIBE_DELETE_RETIRED_FILES), so an update leaves no stale file and an uninstall no install folder.

Var DeleteCredentialsIncomplete
Var DeleteSessionsVaultIncomplete

; The app data folder's name (the Tauri identifier). Tauri's template includes this file before it defines
; BUNDLEID: a macro is expanded where it is inserted (after that define), but a Function body is compiled here, where
; ${BUNDLEID} would stay literal text and the cleanup would look in a folder that does not exist
; A release check keeps this equal to the identifier and keeps ${BUNDLEID} out of Function bodies.
!define VIBE_DATA_ID "app.vibework.desktop"
!define VIBE_BROWSER_HOST "com.vibework.chrome_control"
!define VIBE_BROWSER_EXTENSION "gnlamocfdabihichplpepjpehlbafhng"
!define VIBE_VIDEO_PROFILE_PREFIX "app.vibework.desktop.video."
!define VIBE_VIDEO_PROFILE_MAPPINGS "Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings"

; Files an earlier release installed under $INSTDIR that this version no longer ships. An update installs over the
; old version without uninstalling it (/UPDATE) and the uninstaller deletes only its own version's files, so these
; would stay forever and keep $INSTDIR after an uninstall. The list is exactly the files earlier releases installed
; minus the files this version ships; a release check keeps it in step with the recorded file lists.
; Files one by one (never a recursive delete under $INSTDIR), then their folders deepest first; RMDir without /r
; removes a folder only when it is empty, so a folder this version still uses stays.
!macro VIBE_DELETE_RETIRED_FILES
  ; 0.1.0: the licenses moved to licenses\third_party\ in 0.1.1.
  Delete "$INSTDIR\licenses\jsdiff\LICENSE"
  Delete "$INSTDIR\licenses\remend\LICENSE"
  ; 0.1.0: bytecode from the build machine, never shipped again (package_resources.reject_bytecode).
  Delete "$INSTDIR\plugin\vw\skills\data-analysis\scripts\__pycache__\xlsx_to_csv.cpython-312.pyc"
  ; 0.1.7: the GStreamer plugins-bad notice; 0.1.8 ships no plugins-bad code.
  Delete "$INSTDIR\licenses\third_party\media\COPYRIGHT-gstreamer-plugins-bad"
  RMDir "$INSTDIR\licenses\jsdiff"
  RMDir "$INSTDIR\licenses\remend"
  RMDir "$INSTDIR\plugin\vw\skills\data-analysis\scripts\__pycache__"
  RMDir "$INSTDIR\plugin\vw\skills\data-analysis\scripts"
  RMDir "$INSTDIR\plugin\vw\skills\data-analysis"
  RMDir "$INSTDIR\plugin\vw\skills"
  RMDir "$INSTDIR\plugin\vw"
  RMDir "$INSTDIR\plugin"
  RMDir "$INSTDIR\licenses\third_party\media"
  RMDir "$INSTDIR\licenses\third_party"
  RMDir "$INSTDIR\licenses"
!macroend

!define VIBE_SEND_TO_FILE_VERB "Software\Classes\*\shell\VibeWork.SendTo"
!define VIBE_SEND_TO_FOLDER_VERB "Software\Classes\Directory\shell\VibeWork.SendTo"
; Remembers that the question was asked (Yes or No), so a later manual install does not ask again; the app reads it
; to know the installer asked (else its start screen asks once).
!define VIBE_SEND_TO_ASKED "Software\VibeWork.SendTo"

!macro VIBE_SEND_TO_VERB KEY
  WriteRegStr HKCU "${KEY}" "" "Send to Vibe Work"
  WriteRegStr HKCU "${KEY}" "Icon" "$INSTDIR\${MAINBINARYNAME}.exe"
  WriteRegStr HKCU "${KEY}\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" --attach "%1"'
!macroend

; Interactive installs only: an update (/UPDATE), /P and /S never ask and never add.
!macro VIBE_SEND_TO_ASK
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    Push $0
    ClearErrors
    ReadRegStr $0 HKCU "${VIBE_SEND_TO_ASKED}" "Asked"
    ${If} $0 == ""
      WriteRegStr HKCU "${VIBE_SEND_TO_ASKED}" "Asked" "1"
      MessageBox MB_YESNO|MB_ICONQUESTION "Add 'Send to Vibe Work' to the Windows right-click menu? You can change this later in Settings." /SD IDNO IDNO vibe_send_to_asked
      !insertmacro VIBE_SEND_TO_VERB "${VIBE_SEND_TO_FILE_VERB}"
      !insertmacro VIBE_SEND_TO_VERB "${VIBE_SEND_TO_FOLDER_VERB}"
      SetShellVarContext current
      CreateShortCut "$SENDTO\Vibe Work.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "--attach" "$INSTDIR\${MAINBINARYNAME}.exe" 0 SW_SHOWNORMAL "" "Send to Vibe Work"
      vibe_send_to_asked:
    ${EndIf}
    Pop $0
  ${EndIf}
!macroend

; Every uninstall except an update's: the entries point at the removed program.
!macro VIBE_SEND_TO_REMOVE
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "${VIBE_SEND_TO_FILE_VERB}"
    DeleteRegKey HKCU "${VIBE_SEND_TO_FOLDER_VERB}"
    DeleteRegKey HKCU "${VIBE_SEND_TO_ASKED}"
    SetShellVarContext current
    Delete "$SENDTO\Vibe Work.lnk"
  ${EndIf}
!macroend

; After every install (fresh, replacement or /UPDATE): the new files are in place, the retired ones go.
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro VIBE_DELETE_RETIRED_FILES
  !insertmacro VIBE_SEND_TO_ASK
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Before the template deletes this version's files and runs RMDir "$INSTDIR" (non-recursive).
  !insertmacro VIBE_DELETE_RETIRED_FILES
  ${If} $UpdateMode = 1
  ${OrIf} $PassiveMode = 1
  ${OrIf} ${Silent}
    StrCpy $DeleteAppDataCheckboxState 0
    StrCpy $DeleteSessionsVaultCheckboxState 0
  ${EndIf}
!macroend

!macro NSIS_HOOK_CLEAN_CREDENTIALS
  StrCpy $DeleteCredentialsIncomplete 0
  ${If} $DeleteSessionsVaultCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    ClearErrors
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --clear-install-credentials' $0
    ${If} ${Errors}
    ${OrIf} $0 <> 0
      StrCpy $DeleteCredentialsIncomplete 1
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro VIBE_SEND_TO_REMOVE
  !insertmacro VIBE_BROWSER_HELPER_REMOVE
  !insertmacro VIBE_VIDEO_PROFILES_REMOVE
  !insertmacro VIBE_NOTIFICATION_ICON_REMOVE
  DeleteRegKey HKCU "Software\Classes\CLSID\{E9D28628-47A6-45A7-80C2-2BDC64107A50}"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"
  ${If} $UpdateMode <> 1
    ; Program integration can be the last occupant even when all personal data is kept.
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "$LOCALAPPDATA" "${BUNDLEID}"
  ${EndIf}
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    ${If} $DeleteAppDataCheckboxState = 1
      !insertmacro NSIS_HOOK_DELETE_APP_DATA
    ${EndIf}
    ${If} $DeleteSessionsVaultCheckboxState = 1
      !insertmacro NSIS_HOOK_DELETE_SESSIONS_VAULT
    ${EndIf}
    ${If} $DeleteAppDataCheckboxState = 1
    ${OrIf} $DeleteSessionsVaultCheckboxState = 1
      !insertmacro VIBE_REMOVE_EMPTY_APP_DATA "$APPDATA"
      !insertmacro VIBE_REMOVE_EMPTY_APP_DATA "$LOCALAPPDATA"
    ${EndIf}
  ${EndIf}
!macroend

; The notification image belongs to the removed app, not retained personal data.
!macro VIBE_NOTIFICATION_ICON_REMOVE
  ${If} $UpdateMode <> 1
    !insertmacro VIBE_DELETE_DATA_PATH "$APPDATA" "${BUNDLEID}\notification-icon.png"
    !insertmacro VIBE_DELETE_DATA_PATH "$LOCALAPPDATA" "${BUNDLEID}\notification-icon.png"
  ${EndIf}
!macroend

; Profiles are program integration, independent of the choices to remove personal data.
!macro VIBE_VIDEO_PROFILES_REMOVE
  ${If} $UpdateMode <> 1
    Call un.DeleteVideoProfiles
  ${EndIf}
!macroend

; Snapshot owned monikers before deleting: deletion shifts the registry's enumeration indexes.
; The API removes the profile and its storage; never delete mapping keys or follow a registry-supplied path.
Function un.DeleteVideoProfiles
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  SetRegView 64
  StrCpy $0 0
  StrCpy $1 0
  video_profiles_scan:
    ClearErrors
    EnumRegKey $2 HKCU "${VIBE_VIDEO_PROFILE_MAPPINGS}" $0
    IfErrors video_profiles_collected
    StrCmp $2 "" video_profiles_collected
    IntOp $0 $0 + 1
    ClearErrors
    ReadRegStr $3 HKCU "${VIBE_VIDEO_PROFILE_MAPPINGS}\$2" "Moniker"
    IfErrors video_profiles_scan
    StrCpy $4 $3 27
    StrCmpS $4 "${VIBE_VIDEO_PROFILE_PREFIX}" 0 video_profiles_scan
    Push $3
    IntOp $1 $1 + 1
    Goto video_profiles_scan
  video_profiles_collected:
    SetRegView lastused
  video_profiles_remove:
    IntCmp $1 0 video_profiles_done
    Pop $3
    IntOp $1 $1 - 1
    System::Call 'userenv::DeleteAppContainerProfile(w r3) i.r0'
    ${If} $0 <> 0
      DetailPrint "Could not remove video sandbox $3 ($0)."
    ${EndIf}
    Goto video_profiles_remove
  video_profiles_done:
    Pop $4
    Pop $3
    Pop $2
    Pop $1
    Pop $0
FunctionEnd

!macro VIBE_DELETE_DATA_PATH ROOT RELATIVE
  Push $R8
  StrCpy $R8 0
  Push "${ROOT}"
  Push "${RELATIVE}"
  Call un.DeleteDataPath
  Pop $R8
!macroend

!macro VIBE_REMOVE_EMPTY_DATA_PATH ROOT RELATIVE
  Push $R8
  StrCpy $R8 1
  Push "${ROOT}"
  Push "${RELATIVE}"
  Call un.DeleteDataPath
  Pop $R8
!macroend

; Only for app-managed runtime versions: visit children bottom-up, preserving every file.
!macro VIBE_REMOVE_EMPTY_DATA_TREE ROOT RELATIVE
  Push $R8
  StrCpy $R8 2
  Push "${ROOT}"
  Push "${RELATIVE}"
  Call un.DeleteDataPath
  Pop $R8
!macroend

; Tier 2 owns these runtimes, including uv's directory junction entries, never their external targets.
; The checked root and every ancestor must still be ordinary directories.
!macro VIBE_DELETE_RUNTIME ROOT
  Push $R8
  StrCpy $R8 4
  Push "${ROOT}"
  Push "${BUNDLEID}\vibe"
  Call un.DeleteDataPath
  Pop $R8
!macroend

; Run after both choices: retained data keeps its parents, and linked ancestors are never traversed.
!macro VIBE_REMOVE_EMPTY_APP_DATA ROOT
  ${If} $DeleteSessionsVaultCheckboxState = 1
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\changes\worktrees\records"
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\changes\worktrees"
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\changes\no-hooks"
  ${EndIf}
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\changes\git"
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\changes"
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\pages"
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\work"
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}\preview"
  !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "${ROOT}" "${BUNDLEID}"
!macroend

; A known filename never authorizes removing a directory that happens to use that name.
!macro VIBE_DELETE_SENSITIVE_FILE ROOT RELATIVE
  Push $R8
  StrCpy $R8 3
  Push "${ROOT}"
  Push "${RELATIVE}"
  Call un.DeleteDataPath
  Pop $R8
  ${If} $R9 <> 1
    StrCpy $DeleteSessionsVaultIncomplete 1
  ${EndIf}
!macroend

!macro VIBE_DELETE_SENSITIVE_PATH ROOT RELATIVE
  !insertmacro VIBE_DELETE_DATA_PATH "${ROOT}" "${RELATIVE}"
  ${If} $R9 <> 1
    StrCpy $DeleteSessionsVaultIncomplete 1
  ${EndIf}
!macroend

; Registrations are program integration, removed at every standalone level. Foreign values/children stay.
!macro VIBE_BROWSER_HOST_REMOVE KEY
  ReadRegStr $0 HKCU "${KEY}" ""
  ${If} $0 == "$LOCALAPPDATA\VibeWork\ChromeControl\host\${VIBE_BROWSER_HOST}.json"
    DeleteRegValue HKCU "${KEY}" ""
    DeleteRegKey /ifempty HKCU "${KEY}"
  ${EndIf}
!macroend

!macro VIBE_BROWSER_EXTENSION_REMOVE KEY
  ReadRegStr $0 HKCU "${KEY}" "update_url"
  ${If} $0 == "https://clients2.google.com/service/update2/crx"
    DeleteRegValue HKCU "${KEY}" "update_url"
    DeleteRegKey /ifempty HKCU "${KEY}"
  ${EndIf}
!macroend

!macro VIBE_BROWSER_HELPER_REMOVE
  ${If} $UpdateMode <> 1
    SetShellVarContext current
    Push $0
    Push $1
    ; Chrome/Edge can still hold a native host after the app exits. Only this install's copied executable qualifies.
    nsExec::ExecToStack `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "$$helper = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'VibeWork\ChromeControl\bin\chrome-control.exe'; Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -eq $$helper } | ForEach-Object { $$p = Get-Process -Id $$_.ProcessId -ErrorAction SilentlyContinue; if ($$p) { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue; [void]$$p.WaitForExit(10000) } }"`
    Pop $0
    Pop $1
    !insertmacro VIBE_BROWSER_HOST_REMOVE "Software\Google\Chrome\NativeMessagingHosts\${VIBE_BROWSER_HOST}"
    !insertmacro VIBE_BROWSER_HOST_REMOVE "Software\Microsoft\Edge\NativeMessagingHosts\${VIBE_BROWSER_HOST}"
    !insertmacro VIBE_BROWSER_EXTENSION_REMOVE "Software\Google\Chrome\Extensions\${VIBE_BROWSER_EXTENSION}"
    !insertmacro VIBE_BROWSER_EXTENSION_REMOVE "Software\Microsoft\Edge\Extensions\${VIBE_BROWSER_EXTENSION}"
    !insertmacro VIBE_DELETE_DATA_PATH "$LOCALAPPDATA" "VibeWork\ChromeControl"
    ${If} $R9 <> 1
      DetailPrint "Browser helper removal incomplete."
    ${EndIf}
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "$LOCALAPPDATA" "VibeWork"
    Pop $1
    Pop $0
  ${EndIf}
!macroend

!macro NSIS_HOOK_DELETE_APP_DATA
  SetShellVarContext current
  Push "$APPDATA"
  Push "${BUNDLEID}"
  Call un.DeleteSettingsAndCaches
  ; Installed runtimes belong to the app. Never follow choice.json to an existing install the person selected.
  !insertmacro VIBE_DELETE_RUNTIME "$APPDATA"
  !insertmacro VIBE_DELETE_RUNTIME "$LOCALAPPDATA"
  Push "$LOCALAPPDATA"
  Push "${BUNDLEID}"
  Call un.DeleteSettingsAndCaches
!macroend

!macro NSIS_HOOK_DELETE_SESSIONS_VAULT
  SetShellVarContext current
  StrCpy $DeleteSessionsVaultIncomplete 0
  !insertmacro VIBE_DELETE_SENSITIVE_FILE "$APPDATA" "${BUNDLEID}\plugin-approvals.json"
  !insertmacro VIBE_DELETE_SENSITIVE_FILE "$LOCALAPPDATA" "${BUNDLEID}\plugin-approvals.json"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\vault-settings.json"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\preview\adopted-storage.json"
  Push "$APPDATA"
  Push "${BUNDLEID}"
  Call un.RemoveEmptyRuntimeParents
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$LOCALAPPDATA" "${BUNDLEID}\vault-settings.json"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$LOCALAPPDATA" "${BUNDLEID}\preview\adopted-storage.json"
  Push "$LOCALAPPDATA"
  Push "${BUNDLEID}"
  Call un.RemoveEmptyRuntimeParents
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\sessions"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\temporary-folders"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\projects"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\changes\sessions"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\vibe-home"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "Vibe Work Vault"
  ; The legacy private profile; never consult the selected vault path.
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\vault"
  ; Browser stores only: never remove preview settings/staging or either data root.
  Call un.DeleteBrowserProfiles
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\pages-view"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\files-view"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\EBWebView"
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$LOCALAPPDATA" "${BUNDLEID}\EBWebView"
  ${If} $DeleteCredentialsIncomplete = 1
    MessageBox MB_OK|MB_ICONEXCLAMATION "Vibe Work could not remove its saved credentials. Remove Vibe Work entries in Windows Credential Manager."
  ${EndIf}
  ${If} $DeleteSessionsVaultIncomplete = 1
    MessageBox MB_OK|MB_ICONEXCLAMATION "Vibe Work could not verify complete session, browser sign-in and private vault removal. Some data may remain in Vibe Work app data or the Vibe Work Vault folder."
  ${EndIf}
!macroend

Function un.DeleteBrowserProfiles
  ; StorageKey::hex (vw-preview/ids.rs): only 32 lowercase hex directories.
  ; Includes the stable shared profile; external browser overrides are not read.
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $2 0
  StrCpy $3 "$APPDATA"
  browser_root_check:
    System::Call 'kernel32::GetFileAttributesW(w r3) i .r4 ?e'
    Pop $5
    IntCmp $4 -1 browser_missing browser_failed browser_root_present
  browser_root_present:
    IntOp $5 $4 & 0x400
    IntCmp $5 0 0 browser_failed browser_failed
    IntOp $5 $4 & 0x10
    IntCmp $5 0 browser_failed
    IntCmp $2 2 browser_enumerate
    IntCmp $2 1 browser_preview
    StrCpy $2 1
    StrCpy $3 "$APPDATA\${VIBE_DATA_ID}"
    Goto browser_root_check
  browser_preview:
    StrCpy $2 2
    StrCpy $3 "$APPDATA\${VIBE_DATA_ID}\preview"
    Goto browser_root_check
  browser_enumerate:
    ; WIN32_FIND_DATAW, as in NSIS FileFunc.nsh; explicit Unicode filename fields.
    System::Call '*(i,l,l,l,i,i,i,i,&w260,&w14) p .r0'
    IntCmp $0 0 browser_failed
    StrCpy $3 "$3\*"
    System::Call 'kernel32::FindFirstFileW(w r3, p r0) p .r1 ?e'
    Pop $5
    IntCmp $1 -1 browser_first_failed
  browser_next:
    System::Call '*$0(i .r4,l,l,l,i,i,i,i,&w260 .r2)'
    IntOp $5 $4 & 0x10
    IntCmp $5 0 browser_advance
    StrLen $5 $2
    IntCmp $5 32 0 browser_advance browser_advance
    StrCpy $3 $2
  browser_name_next:
    StrCpy $4 $3 1
    StrCpy $3 $3 "" 1
    StrCmp $4 "" browser_delete
    StrCmpS $4 "0" browser_hex
    StrCmpS $4 "1" browser_hex
    StrCmpS $4 "2" browser_hex
    StrCmpS $4 "3" browser_hex
    StrCmpS $4 "4" browser_hex
    StrCmpS $4 "5" browser_hex
    StrCmpS $4 "6" browser_hex
    StrCmpS $4 "7" browser_hex
    StrCmpS $4 "8" browser_hex
    StrCmpS $4 "9" browser_hex
    StrCmpS $4 "a" browser_hex
    StrCmpS $4 "b" browser_hex
    StrCmpS $4 "c" browser_hex
    StrCmpS $4 "d" browser_hex
    StrCmpS $4 "e" browser_hex
    StrCmpS $4 "f" browser_hex
    Goto browser_advance
  browser_hex:
    Goto browser_name_next
  browser_delete:
    !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${VIBE_DATA_ID}\preview\$2"
  browser_advance:
    ; Capture the native error at the call: only ERROR_NO_MORE_FILES ends a scan.
    System::Call 'kernel32::FindNextFileW(p r1, p r0) i .r4 ?e'
    Pop $5
    IntCmp $4 0 browser_scan_ended
    Goto browser_next
  browser_scan_ended:
    ${If} $5 <> 18
      StrCpy $DeleteSessionsVaultIncomplete 1
    ${EndIf}
  browser_close:
    System::Call 'kernel32::FindClose(p r1) i .r4'
    ${If} $4 = 0
      StrCpy $DeleteSessionsVaultIncomplete 1
    ${EndIf}
  browser_free:
    System::Free $0
    Goto browser_done
  browser_first_failed:
    ${If} $5 <> 2
    ${AndIf} $5 <> 3
      StrCpy $DeleteSessionsVaultIncomplete 1
    ${EndIf}
    Goto browser_free
  browser_missing:
    ${If} $5 = 2
    ${OrIf} $5 = 3
      Goto browser_done
    ${EndIf}
  browser_failed:
    StrCpy $DeleteSessionsVaultIncomplete 1
  browser_done:
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

Function un.DeleteSettingsAndCaches
  ; Base OS directory and identifier are separate so both are checked.
  Exch $6
  Exch 1
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  ; Positive list: unknown children, browser sign-ins and authored state stay.
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\settings.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\keys.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\work\work.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\pages\projects"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\changes\git\settings.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\changes\offers.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\updates.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\updates"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\network"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vault-selection.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\.vault-selection.lock"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\agent-tools"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\debug-api"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\processes"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\shell"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\files-viewer"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\instance.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\instance.lock"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\preview\downloads-staging"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\preview\engine-pool.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\preview\wipe-pending.json"
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\probe"
  ; Display preferences (theme, panel widths, accessibility and notification choices) are the main window's
  ; WebView2 localStorage; only that store goes, the profile's cookies and sign-ins are tier 3.
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\EBWebView\Default\Local Storage"
  Push $0
  Push $6
  Call un.DeleteRuntimeCaches
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Pop $6
FunctionEnd

Function un.DeleteRuntimeCaches
  Push $R7
  Exch 2
  Exch 1
  StrCpy $R7 0
  Call un.VisitRuntimeParents
  Pop $R7
FunctionEnd

Function un.RemoveEmptyRuntimeParents
  Push $R7
  Exch 2
  Exch 1
  StrCpy $R7 1
  Call un.VisitRuntimeParents
  Pop $R7
FunctionEnd

Function un.VisitRuntimeParents
  ; Called with the mode in R7 and the same checked data root/relative identifier as settings cleanup.
  Exch $6
  Exch 1
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  ; Managed runtime version caches only; never remove the runtime/home wholesale.
  StrCpy $3 "$0"
  System::Call 'kernel32::GetFileAttributesW(w r3) i .r4'
  IntCmp $4 -1 caches_done caches_done
  IntOp $5 $4 & 0x400
  IntCmp $5 0 0 caches_done caches_done
  StrCpy $3 "$0\$6"
  System::Call 'kernel32::GetFileAttributesW(w r3) i .r4'
  IntCmp $4 -1 caches_done caches_done
  IntOp $5 $4 & 0x400
  IntCmp $5 0 0 caches_done caches_done
  StrCpy $3 "$0\$6\vibe"
  System::Call 'kernel32::GetFileAttributesW(w r3) i .r4'
  IntCmp $4 -1 caches_done caches_done
  IntOp $5 $4 & 0x400
  IntCmp $5 0 0 caches_done caches_done
  ClearErrors
  FindFirst $1 $2 "$0\$6\vibe\*"
  IfErrors caches_done
  caches_next:
    StrCmp $2 "" caches_close
    StrCmp $2 "." caches_advance
    StrCmp $2 ".." caches_advance
    ; Layout::new accepts only nonempty groups of digits separated by dots.
    StrCpy $3 $2
    StrCpy $5 1
  version_next:
    StrCpy $4 $3 1
    StrCpy $3 $3 "" 1
    StrCmp $4 "" version_done
    StrCmp $4 "." version_dot
    StrCmp $4 "0" version_digit
    StrCmp $4 "1" version_digit
    StrCmp $4 "2" version_digit
    StrCmp $4 "3" version_digit
    StrCmp $4 "4" version_digit
    StrCmp $4 "5" version_digit
    StrCmp $4 "6" version_digit
    StrCmp $4 "7" version_digit
    StrCmp $4 "8" version_digit
    StrCmp $4 "9" version_digit
    Goto caches_advance
  version_dot:
    StrCmp $5 1 caches_advance
    StrCpy $5 1
    Goto version_next
  version_digit:
    StrCpy $5 0
    Goto version_next
  version_done:
    StrCmp $5 1 caches_advance
    ${If} $R7 = 1
      !insertmacro VIBE_REMOVE_EMPTY_DATA_TREE "$0" "$6\vibe\$2"
    ${Else}
      !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\$2\cache"
      !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\$2\dl"
    ${EndIf}
  caches_advance:
    ClearErrors
    FindNext $1 $2
    IfErrors caches_close caches_next
  caches_close:
    FindClose $1
  caches_done:
  ${If} $R7 = 1
    !insertmacro VIBE_REMOVE_EMPTY_DATA_PATH "$0" "$6\vibe"
  ${EndIf}
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Pop $6
FunctionEnd

Function un.DeleteDataPath
  ; Two inputs: OS data root, then a relative literal from the lists above.
  ; $R9 = 1 only when absence is verified; missing ancestors also prove absence.
  ; R8 modes: 0 deletes a tree, 1 an empty folder, 2 empty descendants, 3 a file only.
  ; Mode 4 deletes owned runtimes and unlinks child directory junctions without following them.
  ; All modes share the ancestor/reparse checks; macros save/restore R8.
  Exch $1
  Exch 1
  Exch $0
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $R9 0
  StrCpy $3 "$0"
  data_check:
  System::Call 'kernel32::GetFileAttributesW(w r3) i .r4 ?e'
  Pop $5
  IntCmp $4 -1 data_missing data_done data_present
  data_present:
    IntOp $5 $4 & 0x400
    IntCmp $5 0 0 data_done data_done
    StrCmp $1 "" data_delete
    IntOp $5 $4 & 0x10
    IntCmp $5 0 data_done
    StrCpy $3 "$3\"
  data_append:
    StrCpy $2 $1 1
    StrCpy $1 $1 "" 1
    StrCmp $2 "" data_check
    StrCmp $2 "\" data_check
    StrCpy $3 "$3$2"
    Goto data_append
  data_delete:
    StrCmp $R8 1 data_empty
    StrCmp $R8 3 data_file
    Push "$3"
    Call un.DeleteTreeNoReparse
    Pop $R9
    Goto data_verify
  data_file:
    Delete "$3"
    Goto data_verify
  data_empty:
    RMDir "$3"
  data_verify:
    StrCpy $R9 0
    System::Call 'kernel32::GetFileAttributesW(w r3) i .r4 ?e'
    Pop $5
    IntCmp $4 -1 data_missing data_done data_done
  data_missing:
    ${If} $5 = 2
    ${OrIf} $5 = 3
      StrCpy $R9 1
    ${EndIf}
  data_done:
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $0
  Pop $1
FunctionEnd

Function un.DeleteTreeNoReparse
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  StrCpy $3 "$0"
  System::Call 'kernel32::GetFileAttributesW(w r3) i .r4'
  IntCmp $4 -1 tree_done tree_done
  IntOp $5 $4 & 0x400
  IntCmp $5 0 tree_plain tree_link tree_link
  tree_link:
    StrCmp $R8 4 0 tree_done
    IntOp $5 $4 & 0x10
    IntCmp $5 0 tree_done
    ; RemoveDirectory unlinks the junction itself, even if its target is nonempty. Never enumerate the target.
    System::Call 'kernel32::RemoveDirectoryW(w r0) i .r5'
    Goto tree_done
  tree_plain:
  IntOp $5 $4 & 0x10
  IntCmp $5 0 tree_file tree_dir tree_dir
  tree_file:
    StrCmp $R8 2 tree_done
    Delete "$0"
    Goto tree_done
  tree_dir:
    ClearErrors
    FindFirst $1 $2 "$0\*"
    IfErrors tree_done
  tree_next:
    StrCmp $2 "" tree_close
    StrCmp $2 "." tree_advance
    StrCmp $2 ".." tree_advance
    Push "$0\$2"
    Call un.DeleteTreeNoReparse
    Pop $3
  tree_advance:
    ClearErrors
    FindNext $1 $2
    IfErrors tree_close tree_next
  tree_close:
    FindClose $1
    RMDir "$0"
  tree_done:
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd
