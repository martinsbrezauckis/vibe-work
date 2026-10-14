; Vibe Work NSIS uninstall data handlers. Included by the pinned Tauri 2.10.1 template.
; Called after standalone GUI removal; updates, /P and /S retain all user data.
; Invariant: tier 2 deletes only listed settings/preferences/caches. Only the
; separate tier-3 choice deletes managed sessions, browser sign-ins and the private/legacy vault.
; Every app-owned ancestor is checked; links/reparse points are never traversed.

Var DeleteSessionsVaultIncomplete

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode = 1
  ${OrIf} $PassiveMode = 1
  ${OrIf} ${Silent}
    StrCpy $DeleteAppDataCheckboxState 0
    StrCpy $DeleteSessionsVaultCheckboxState 0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    ${If} $DeleteAppDataCheckboxState = 1
      !insertmacro NSIS_HOOK_DELETE_APP_DATA
    ${EndIf}
    ${If} $DeleteSessionsVaultCheckboxState = 1
      !insertmacro NSIS_HOOK_DELETE_SESSIONS_VAULT
    ${EndIf}
  ${EndIf}
!macroend

!macro VIBE_DELETE_DATA_PATH ROOT RELATIVE
  Push "${ROOT}"
  Push "${RELATIVE}"
  Call un.DeleteDataPath
!macroend

!macro VIBE_DELETE_SENSITIVE_PATH ROOT RELATIVE
  !insertmacro VIBE_DELETE_DATA_PATH "${ROOT}" "${RELATIVE}"
  ${If} $R9 <> 1
    StrCpy $DeleteSessionsVaultIncomplete 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_DELETE_APP_DATA
  SetShellVarContext current
  Push "$APPDATA"
  Push "${BUNDLEID}"
  Call un.DeleteSettingsAndCaches
  Push "$LOCALAPPDATA"
  Push "${BUNDLEID}"
  Call un.DeleteSettingsAndCaches
!macroend

!macro NSIS_HOOK_DELETE_SESSIONS_VAULT
  SetShellVarContext current
  StrCpy $DeleteSessionsVaultIncomplete 0
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\sessions"
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
  !insertmacro VIBE_DELETE_SENSITIVE_PATH "$LOCALAPPDATA" "${BUNDLEID}\EBWebView"
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
    StrCpy $3 "$APPDATA\${BUNDLEID}"
    Goto browser_root_check
  browser_preview:
    StrCpy $2 2
    StrCpy $3 "$APPDATA\${BUNDLEID}\preview"
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
    !insertmacro VIBE_DELETE_SENSITIVE_PATH "$APPDATA" "${BUNDLEID}\preview\$2"
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
  !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\probe"
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
    !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\$2\cache"
    !insertmacro VIBE_DELETE_DATA_PATH "$0" "$6\vibe\$2\dl"
  caches_advance:
    ClearErrors
    FindNext $1 $2
    IfErrors caches_close caches_next
  caches_close:
    FindClose $1
  caches_done:
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
    Push "$3"
    Call un.DeleteTreeNoReparse
    Pop $R9
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
  IntCmp $5 0 0 tree_done tree_done
  IntOp $5 $4 & 0x10
  IntCmp $5 0 tree_file tree_dir tree_dir
  tree_file:
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
