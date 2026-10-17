//! A type-erased `Confirmer`, so `Core` is one concrete type that both clients (Tauri bridge,
//! Debug API) can hold without being generic over the dialog implementation.
//!
//! Depends on: vw-gate (`Confirmer`, `Confirmation`, `ConfirmationRequest`), `dialog_texts` (sentences the
//! core wrote for one exact call). Called by: `core` (wraps the app's confirmer), the gate (asks
//! it).
//! Invariants:
//! - **Debug API requests:** a request whose `requester` is
//!   [`Requester::DebugApi`] is answered `Approved` here, without a dialog, and logged (method only, never
//!   params), except the vault dialogs below. The Debug API is the operator's console with the operator's access, so a
//!   Full access escalation, a proxy or certificate change, a provider consent and the rest are answered here too.
//!   This is the one choke point for every confirmation the core and the gate ask. Who may carry that requester is
//!   decided by the transport: only the Debug API socket (bearer token) and the window while it serves a Debug API
//!   UI-control request (`vw-host` `execute_for_ui_control`).
//! - **Vault dialog exception:** a request whose method is one of the vault's dialogs
//!   (`vault_scope::DIALOG_METHODS`: giving an MCP server a vault secret, storing a captured key, running a command
//!   with a secret, filling a vault login's password into a page) always goes to the native dialog,
//!   even when the Debug API asked. The Debug API may start the flow; the user's answer in the window
//!   decides whether a value leaves the vault. The agent never answers a vault dialog.
//! - **Core requests:** a request with [`Requester::Core`] (the core finishing a step the user chose earlier: deleting
//!   a temporary session at quit or at the next start) is answered `Approved` the same way. Only core code builds
//!   such a request; no client origin maps to it (`command::requester`).
//! - Every other request: the wrapped confirmer is asked exactly once and its answer is returned unchanged
//!   (except `ApprovedAlways` to a request that did not offer it, which becomes `Approved`); it
//!   must fail closed (`Declined`) when no dialog can be shown (vw-gate rule). A Debug API request that offers
//!   "always" is answered `Approved` (once): the client acting as the user asks again next time.
//! - Provider permission grants wider than basic identity use `vibe-work/providerConsent`: the dialog is for the
//!   user and the agent; a Debug API browser action answers it here like any other confirmation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tracing::info;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_session::SessionStore;

use crate::dialog_texts::DialogTexts;
use crate::names::Names;

type Asking = Pin<Box<dyn Future<Output = Confirmation> + Send>>;

/// Holds any `Confirmer` behind one function pointer, and the sentences the core registered for exact calls.
#[derive(Clone)]
pub(crate) struct DynConfirmer(
    Arc<dyn Fn(ConfirmationRequest) -> Asking + Send + Sync>,
    Arc<DialogTexts>,
);

impl DynConfirmer {
    /// `names` and `store` let the dialog text name a session by its title instead of its id. Sentences the
    /// core writes for one exact call (a worktree removal naming its files) go through
    /// [`DynConfirmer::texts`].
    pub(crate) fn new<C: Confirmer + 'static>(
        confirmer: C,
        names: Arc<Names>,
        store: SessionStore,
    ) -> Self {
        let confirmer = Arc::new(confirmer);
        let texts = Arc::new(DialogTexts::default());
        let registry = texts.clone();
        let ask = Arc::new(move |mut request: ConfirmationRequest| {
            // a vault dialog (a key to an MCP server, a captured key stored, a command run with a
            // secret, a vault login filled) is never answered for the Debug API: the user's own answer in the
            // native dialog decides. Nothing else waits for a person when the Debug API asks.
            if request.requester == Requester::DebugApi
                && !crate::vault_scope::DIALOG_METHODS.contains(&request.method.as_str())
            {
                // The Debug API is the user (module docs): its own request is its consent.
                info!(
                    method = %request.method,
                    "confirmation answered for the Debug API (acts as the user, no dialog)"
                );
                return Box::pin(std::future::ready(Confirmation::Approved)) as Asking;
            }
            if request.requester == Requester::Core {
                // A step the user chose earlier (deleting a temporary session); only the core sets it.
                info!(
                    method = %request.method,
                    "confirmation answered by the core (a step the user chose earlier, no dialog)"
                );
                return Box::pin(std::future::ready(Confirmation::Approved)) as Asking;
            }
            let confirmer = confirmer.clone();
            if let Some(text) = texts.take(&request.method, &request.params) {
                text.apply(&mut request.summary);
            } else if request.method == "session/agent/update"
                && vw_gate::full_access_escalation(&request.method, &request.params)
            {
                let title = request
                    .params
                    .get("sessionId")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|id| {
                        store
                            .view(id)
                            .and_then(|view| {
                                let session = view.state.get("session")?;
                                session
                                    .get("title")
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|title| !title.trim().is_empty())
                                    .map(str::to_owned)
                            })
                            .or_else(|| names.title_of(&store, id))
                    })
                    .unwrap_or_else(|| "this chat".to_owned());
                request.summary = format!(
                    "Let the agent in {} run every tool without asking ({})?",
                    vw_gate::dialog_quote(&title),
                    vw_vocabulary::label("accessModes", "auto-approve")
                );
            } else if request.method == "workspace/git/worktrees/remove" {
                // A removal the core did not prepare (a raw `call`): still a specific sentence.
                request.summary = crate::worktrees_remove::describe_raw(&request.params);
            } else if request.method == "session/delete" {
                let title = request
                    .params
                    .get("sessionId")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|id| names.title_of(&store, id));
                if let Some(title) = title {
                    // The title is the session's (agent- or user-written): sanitized and quoted.
                    let id = request
                        .params
                        .get("sessionId")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    request.summary = format!(
                        "Delete session {} (ID {}) and its history permanently (the Vibe CLI loses it too)?",
                        vw_gate::dialog_quote(&title),
                        vw_gate::dialog_quote(id),
                    );
                }
            } else if let Some(text) =
                crate::connector_consent::describe(&request.method, &request.params)
            {
                // The gate's generic sentence becomes one that names the server or connector.
                // Who asked is the request's `requester` field (the dialog shows it first), not this text.
                request.summary = text;
            }
            Box::pin(async move {
                let offered = request.always.is_some();
                match confirmer.confirm(&request).await {
                    // "always" is an answer only to a request that offered it.
                    Confirmation::ApprovedAlways if !offered => Confirmation::Approved,
                    answer => answer,
                }
            }) as Asking
        });
        Self(ask, registry)
    }

    /// The registry of sentences for exact calls (shared with the core).
    pub(crate) fn texts(&self) -> Arc<DialogTexts> {
        self.1.clone()
    }
}

impl Confirmer for DynConfirmer {
    fn confirm(&self, request: &ConfirmationRequest) -> impl Future<Output = Confirmation> + Send {
        (self.0)(request.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;

    /// A native dialog that records the methods it is shown and declines every one.
    #[derive(Clone, Default)]
    struct Dialog(Arc<Mutex<Vec<String>>>);

    impl Dialog {
        fn shown(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Confirmer for Dialog {
        fn confirm(
            &self,
            request: &ConfirmationRequest,
        ) -> impl Future<Output = Confirmation> + Send {
            self.0.lock().unwrap().push(request.method.clone());
            std::future::ready(Confirmation::Declined)
        }
    }

    fn request(requester: Requester, method: &str, params: Value) -> ConfirmationRequest {
        ConfirmationRequest {
            method: method.to_owned(),
            summary: "Allow this?".to_owned(),
            params,
            requester,
            always: None,
        }
    }

    /// The product rule: through the Debug API only vault actions wait for a person. A vault dialog method goes to
    /// the native dialog (which here declines it); every other confirmation, a Full access escalation included, is
    /// answered by the core with no dialog; the window's own request for the same Full access still goes to the dialog.
    #[tokio::test]
    async fn the_debug_api_is_answered_for_itself_except_for_the_vault_dialogs() {
        let dialog = Dialog::default();
        let confirmer = DynConfirmer::new(
            dialog.clone(),
            Arc::new(Names::default()),
            vw_session::SessionStore::new(),
        );
        for method in crate::vault_scope::DIALOG_METHODS {
            let answer = confirmer
                .confirm(&request(Requester::DebugApi, method, json!({})))
                .await;
            assert_eq!(
                answer,
                Confirmation::Declined,
                "{method} must reach the dialog"
            );
        }
        assert_eq!(
            dialog.shown().len(),
            crate::vault_scope::DIALOG_METHODS.len()
        );

        let full_access = json!({"sessionId": "s1", "agentName": "auto-approve"});
        let default_profile = json!({"sessionId": "s1", "ops": [
            {"op": "set", "path": "/default_agent", "value": "auto-approve"}]});
        let before = dialog.shown().len();
        for (method, params) in [
            ("session/agent/update", full_access.clone()),
            ("config/write", default_profile),
            ("config/proxy/write", json!({})),
            (crate::browser_consent::METHOD, json!({})),
            ("vibe-work/projectInstructions/delete", json!({})),
            ("session/delete", json!({"sessionId": "s1"})),
        ] {
            let answer = confirmer
                .confirm(&request(Requester::DebugApi, method, params))
                .await;
            assert_eq!(answer, Confirmation::Approved, "{method} must not wait");
        }
        assert_eq!(dialog.shown().len(), before, "no dialog for the Debug API");

        let window = confirmer
            .confirm(&request(
                Requester::Window,
                "session/agent/update",
                full_access,
            ))
            .await;
        assert_eq!(window, Confirmation::Declined);
        assert_eq!(
            dialog.shown().last().map(String::as_str),
            Some("session/agent/update")
        );
    }
}
