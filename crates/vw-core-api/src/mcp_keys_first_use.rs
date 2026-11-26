//! A key server's first use: the agent's first forwarded request to a key server that holds no grant asks the
//! person with the vault's own key dialog (the same dialog as Tools > Allow key, requester the agent) instead of
//! refusing at once. Typical case: a server brought from another agent's setup, whose key went to the vault with
//! permission Ask; also a server allowed only "until Vibe Work closes" in an earlier run.
//!
//! Rules:
//! - Asked only for a record that was never revoked (`revokedMs == 0`). A revoke is the person's own "stop", so a
//!   revoked server is refused at once, as before, and Tools > Allow key gives it back.
//! - One dialog per server and run. Requests that arrive while it is open wait for the same answer; after the
//!   answer (also "Don't allow" or a failure) the server is not asked about again in this run, so a refused server
//!   cannot open a dialog on every request (Vibe retries discovery on each turn).
//! - The dialog runs in its own task: Vibe gives a server 10 s to start, and a request it gives up on must not take
//!   the person's dialog or answer with it. The answer is stored as any Allow key answer is (`allow_binding`), so
//!   the next request finds the grant. After a grant every open session re-reads its MCP servers (as after Tools >
//!   Allow key): the chat whose start raised the dialog gets the server even when Vibe gave up on it meanwhile.
//! - Only the person answers: the method is one of `vault_scope::DIALOG_METHODS`, which the confirmer never
//!   answers for the Debug API, and the agent's request cannot choose the answer.
//!
//! Depends on: the parent's `allow_binding` (dialog, revoke race, grant), `connectors::refresh_servers`, tokio (task,
//! watch channel).
//! Called by: `mcp_keys_runtime` (the forwarder's resolver), after the policy checks and before the key is read.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;
use vw_keys::McpBinding;

use crate::core::Shared;
use crate::error::ErrorCode;

/// Binding id -> the answer of this run's first-use dialog (`None` while it is open; `Some(true)` when a grant was
/// given). An entry stays for the whole run: it is how "asked once" is remembered.
#[derive(Default)]
pub(crate) struct FirstUse {
    asked: Mutex<HashMap<String, watch::Receiver<Option<bool>>>>,
}

impl FirstUse {
    fn asked(&self) -> std::sync::MutexGuard<'_, HashMap<String, watch::Receiver<Option<bool>>>> {
        self.asked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// True when the person allowed `binding`'s key at this first use (the caller still re-reads the record and checks
/// the grant). False when the record was revoked, this run already asked and the answer was not a grant, or the
/// dialog was declined or failed.
pub(crate) async fn ask(shared: &Arc<Shared>, binding: &McpBinding) -> bool {
    if binding.always || binding.revoked_ms != 0 {
        return false;
    }
    let mut answer = {
        let state = &shared.connectors.mcp_keys.first_use;
        let mut asked = state.asked();
        if let Some(answer) = asked.get(&binding.id) {
            answer.clone()
        } else {
            let (tell, answer) = watch::channel(None);
            asked.insert(binding.id.clone(), answer.clone());
            let shared = Arc::clone(shared);
            let binding = binding.clone();
            tokio::spawn(async move {
                let granted = match super::allow_binding(
                    &shared,
                    vw_gate::Requester::Agent,
                    binding,
                )
                .await
                {
                    Ok(()) => true,
                    Err(error) => {
                        if error.code != ErrorCode::ConfirmationDeclined {
                            tracing::warn!(code = ?error.code, "mcp key first use: not allowed");
                        }
                        false
                    }
                };
                let _ = tell.send(Some(granted));
                if granted {
                    // Vibe may have given up on the server while the dialog was open, and does not retry it for the
                    // chat that started it: every open session re-reads its servers, as after Tools > Allow key.
                    crate::connectors::refresh_servers(&shared).await;
                }
            });
            answer
        }
    };
    match answer.wait_for(Option::is_some).await {
        Ok(value) => *value == Some(true),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::refresh_notice_tests::{
        attach_mock_channel, count_server_refreshes, drain_events, memory_keys, start_with_keys,
        tools_alerts,
    };

    #[tokio::test]
    async fn first_use_key_grant_while_off_with_inline_paused_stdio_refreshes_without_alert() {
        let key_root = std::env::temp_dir().join(format!(
            "vw-refresh-first-use-keys-{}-{}",
            std::process::id(),
            vw_changes::now_ms()
        ));
        let keys = memory_keys(&key_root).await;
        let core = start_with_keys(
            "refresh-note-first-use",
            Some(keys.clone()),
            false,
            "mcp_servers = [{ name = \"inline-stdio\", transport = \"stdio\", command = \"/nonexistent/inline\", disabled = true }]\n",
        )
        .await;
        let imported = crate::mcp_keys::import_key_server(
            &core.shared,
            "wiki",
            "https://wiki.example.com/mcp",
            "x-api-key",
            "wiki-secret-123".to_owned(),
        )
        .await
        .unwrap();
        let binding = keys.mcp_binding(imported.binding).await.unwrap().unwrap();

        let calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let _p1 = attach_mock_channel(&core, "s1", 1, calls.clone());
        let mut sub = core.subscribe(None);
        let _ = drain_events(&mut sub).await;

        assert!(super::ask(&core.shared, &binding).await);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && count_server_refreshes(&calls) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            count_server_refreshes(&calls),
            1,
            "first-use grant must refresh the open chat"
        );
        assert_eq!(
            tools_alerts(&drain_events(&mut sub).await),
            Vec::<(Option<String>, String)>::new()
        );

        core.shutdown().await;
        let _ = std::fs::remove_dir_all(&key_root);
    }
}
