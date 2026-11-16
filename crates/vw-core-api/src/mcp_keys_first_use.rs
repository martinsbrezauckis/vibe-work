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
