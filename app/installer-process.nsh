; Vibe Work installer process-exit check. Based on tauri-bundler 2.10.1 utils.nsh.
; Upstream authors: Tauri Programme within The Commons Conservancy; SPDX-License-Identifier: Apache-2.0 OR MIT.
; Called by installer.nsi for install/removal; only /UPDATE retries a shutdown error while the old process exits.
; Uses Restart Manager from the pinned template; all other prompts and failure paths retain upstream behavior.

Var VibeUpdateExitPolls

!macro VibeCheckIfAppIsRunning executablePath productName
  !define UniqueID ${__LINE__}

  ; Replace {{product_name}} placeholder in the messages with the passed product name
  nsis_tauri_utils::StrReplace "$(appRunning)" "{{product_name}}" "${productName}"
  Pop $R1
  nsis_tauri_utils::StrReplace "$(appRunningOkKill)" "{{product_name}}" "${productName}"
  Pop $R2
  nsis_tauri_utils::StrReplace "$(failedToKillApp)" "{{product_name}}" "${productName}"
  Pop $R3

  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 != ""
    !insertmacro RestartManager_RegisterFile $R0 "${executablePath}"
    ${If} $0 = 0
      System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
      ; ERROR_SUCCESS means no running processes, ERROR_MORE_DATA means there're some
      ${If} $0 = ${ERROR_MORE_DATA}
        IfSilent kill_${UniqueID} 0
        ${IfThen} $PassiveMode != 1 ${|} MessageBox MB_OKCANCEL $R2 IDOK kill_${UniqueID} IDCANCEL cancel_${UniqueID} ${|}
        kill_${UniqueID}:
          ; Call manually here to get the result
          ; !insertmacro RestartManager_Shutdown $R0
          System::Call 'RSTRTMGR::RmShutdown(p R0, i ${RmForceShutdown}, p 0) i .r0'
          ${If} $0 = 0
            Goto app_check_done_${UniqueID}
          ${Else}
            ; RmShutdown can fail while the updater's old process is already exiting. Its error is not proof
            ; that the executable is still held: re-query the registered file before aborting the update.
            ; Standalone installs keep the upstream prompt/error behavior. Never wait forever or ignore an
            ; unreadable process list; after at most 100 checks (9.9 s), keep the original failure.
            ${If} $UpdateMode = 1
              StrCpy $VibeUpdateExitPolls 100
              ${Do}
                StrCpy $1 0
                StrCpy $2 0
                System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r0'
                ${If} $0 = 0
                  Goto app_check_done_${UniqueID}
                ${ElseIf} $0 != ${ERROR_MORE_DATA}
                  ${Break}
                ${EndIf}
                IntOp $VibeUpdateExitPolls $VibeUpdateExitPolls - 1
                ${If} $VibeUpdateExitPolls = 0
                  ${Break}
                ${EndIf}
                Sleep 100
              ${Loop}
            ${EndIf}
            IfSilent silent_${UniqueID} ui_${UniqueID}
            silent_${UniqueID}:
              System::Call 'kernel32::AttachConsole(i -1)i.r0'
              ${If} $0 != 0
                System::Call 'kernel32::GetStdHandle(i -11)i.r0'
                System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
                FileWrite $0 "$R1$\n"
              ${EndIf}
              !insertmacro RestartManager_EndSession $R0
              Abort
            ui_${UniqueID}:
              !insertmacro RestartManager_EndSession $R0
              Abort $R3
          ${EndIf}
        cancel_${UniqueID}:
          !insertmacro RestartManager_EndSession $R0
          Abort $R1
        app_check_done_${UniqueID}:
      ${EndIf}
    ${EndIf}
    !insertmacro RestartManager_EndSession $R0
  ${EndIf}

  !undef UniqueID
!macroend
