//! Connectors and MCP servers in the core:
//! the `connectors` command. Mistral-hosted connectors are listed, signed in through Mistral and turned
//! off or on; MCP servers that sign in with OAuth are added by address, signed in through Vibe, signed
//! out, turned off and removed. Servers that need an API key are added and allowed through
//! `mcp_keys` (the key stays in the vault; Vibe Work's forwarder adds it).
//!
//! Depends on: `connectors_view` (types), `connectors_project` (Vibe's replies -> views),
//! `connectors_config` (servers in Vibe Work's own config), `connector_consent` (dialog texts), the gate,
//! `channels`, `dispatch::forward`. Called by: `dispatch` (`Command::Connectors`).
//! Invariants:
//! - Every call to Vibe goes through `Core::forward` (the gate, the Debug API dialog for changes, the
//!   channel rules) or, for the follow-up refreshes and the reads of an already open session, through the
//!   gate's forwarded reads. Nothing here talks to the transport.
//! - Catalog reads, adds, sign-ins of servers, sign-outs, removals and toggles go to the lobby: Vibe writes
//!   its user config once for every session. Afterwards each session with an open channel is asked to
//!   refresh, so live sessions converge; a failing refresh is logged, never fatal.
//! - A sign-in from the app's window needs no native dialog (`TRUSTED_SURFACE_METHODS`); from the Debug
//!   API it always asks. Adding a server always asks (a new remote endpoint).
//! - At most one server sign-in runs (Vibe's callback port is fixed). It runs as an owned task that
//!   holds only a weak reference to the core between steps and is aborted when the core goes away.
//! - Nothing from a sign-in other than the page address passes through here: the browser gives the code to
//!   Mistral or to Vibe's loopback port, and the token stays at Mistral or in Vibe's keyring.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_gate::{ConfirmationToken, GateError, TRUSTED_SURFACE_METHODS};
use vw_keys::KeyPresence;

use crate::command::{CommandResult, Origin};
use crate::connectors_config::configured_servers;
use crate::connectors_project::{
    catalog_load, connectors, merge_plugin_sources, merge_session_connectors, merge_session_servers,
};
use crate::connectors_view::{ConnectorsRequest, ConnectorsResult, ConnectorsView};
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;

/// The running server sign-in and whether the catalog was loaded once in this run.
#[derive(Default)]
pub(crate) struct ConnectorsState {
    signing_in: Mutex<Option<(String, JoinHandle<()>)>>,
    /// Set after the first automatic catalog load, so a key-less run does not ask Mistral on every read.
    catalog_tried: AtomicBool,
    /// Names removed this run; an active Vibe turn cannot refresh its MCP tools until it ends.
    removed: Mutex<HashSet<String>>,
    /// MCP servers that need a key (`mcp_keys.rs`): grants until Vibe Work closes and use counters.
    pub(crate) mcp_keys: crate::mcp_keys::McpKeysState,
}

impl ConnectorsState {
    pub(crate) fn removed_tool(&self, tool: &str) -> bool {
        self.removed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            // Marketplace names allow dashes; Vibe normalizes them in MCP tool groups.
            .any(|name| tool.starts_with(&format!("mcp_{}.", name.replace('-', "_"))))
    }

    /// Rechecks removal at the user's answer: the pre-tool hook ran before Vibe showed the card.
    pub(crate) fn callback_answer(
        &self,
        session: &str,
        id: &str,
        gate: &vw_gate::Gate<impl vw_gate::Transport, impl vw_gate::Confirmer>,
        output: Value,
    ) -> Result<Value, CoreError> {
        gate.validate_callback_answer(session, id, &output)
            .map_err(|error| CoreError::from_gate("callback/result", &error))?;
        Ok(
            if output["type"] == "approval"
                // Every yes reaches here as `approve` (`ask::cards` sends no other granting decision).
                && output["decision"]["type"] == "approve"
                && gate.open_callbacks().iter().any(|callback| {
                    callback.id == id
                        && callback.session_id == session
                        && callback
                            .tool_name
                            .as_deref()
                            .is_some_and(|tool| self.removed_tool(tool))
                })
            {
                json!({"type": "approval", "decision": {"type": "deny"},
                "feedback": "This connector was removed."})
            } else {
                output
            },
        )
    }

    pub(crate) fn mark_removed(&self, name: &str, removed: bool) {
        let mut names = self.removed.lock().unwrap_or_else(PoisonError::into_inner);
        if removed {
            names.insert(name.to_owned());
        } else {
            names.remove(name);
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<(String, JoinHandle<()>)>> {
        self.signing_in
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The server whose sign-in is running.
    fn running(&self) -> Option<String> {
        self.slot()
            .as_ref()
            .filter(|(_, task)| !task.is_finished())
            .map(|(name, _)| name.clone())
    }
}

impl Drop for ConnectorsState {
    fn drop(&mut self) {
        if let Some((_, task)) = self.slot().take() {
            task.abort();
        }
    }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn checked_name(name: &str) -> Result<(), CoreError> {
    if name.trim().is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        return Err(invalid(
            "the name must have 1 to 128 characters and no control characters",
        ));
    }
    Ok(())
}

/// Which refresh live sessions need after a change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Converge {
    Connectors,
    Servers,
}

impl Core {
    /// Runs one `connectors` request for `origin`.
    ///
    /// # Errors
    /// Gate refusals, `confirmationDeclined`, `invalidParams` (a refused address, a bad name, no session
    /// for a Mistral sign-in), `denied` with `serverCode: "signInRunning"` while a server sign-in runs,
    /// and Vibe's own errors (`serverError` with Vibe's code and message).
    pub(crate) async fn connectors(
        &self,
        origin: Origin,
        request: ConnectorsRequest,
    ) -> Result<CommandResult, CoreError> {
        let result = match request {
            ConnectorsRequest::Read { session_id } => ConnectorsResult::View {
                view: self.connectors_view(origin, session_id).await,
            },
            ConnectorsRequest::Refresh { session_id } => {
                self.refresh_catalog(origin).await;
                self.converge(Converge::Connectors).await;
                ConnectorsResult::View {
                    view: self.connectors_view(origin, session_id).await,
                }
            }
            ConnectorsRequest::SignIn { session_id, alias } => {
                checked_name(&alias)?;
                if session_id.is_empty() {
                    return Err(invalid(
                        "a Mistral connector signs in through a session: open a session first",
                    ));
                }
                let params = json!({"sessionId": session_id, "alias": alias});
                let reply = self
                    .signed_call(origin, "connector_catalog/auth/request", params)
                    .await?;
                let request_id = reply
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                ConnectorsResult::SignInStarted {
                    alias,
                    session_id,
                    request_id,
                }
            }
            ConnectorsRequest::SetConnectorOff { alias, off } => {
                checked_name(&alias)?;
                let params = json!({"alias": alias, "disabled": off});
                self.forward(origin, "connector_catalog/toggle", params, None)
                    .await?;
                self.converge(Converge::Connectors).await;
                self.view_result(origin).await
            }
            ConnectorsRequest::AddServer { url, name } => {
                self.add_server(origin, url, name).await?
            }
            ConnectorsRequest::SignInServer { name } => {
                checked_name(&name)?;
                self.start_server_sign_in(origin, name).await?
            }
            ConnectorsRequest::CancelSignIn {} => {
                let taken = self.shared.connectors.slot().take();
                if let Some((name, task)) = taken {
                    task.abort();
                    self.shared.emit(CoreEvent::ConnectorSignIn {
                        name,
                        outcome: "cancelled".to_owned(),
                        message: None,
                    });
                }
                self.view_result(origin).await
            }
            ConnectorsRequest::SignOutServer { name } => {
                self.server_change(origin, "mcp_catalog/logout", json!({"name": name}))
                    .await?
            }
            ConnectorsRequest::RemoveServer { name } => {
                let result = self
                    .server_change(origin, "mcp_catalog/remove", json!({"name": name.clone()}))
                    .await?;
                // A key server's vault record goes with it.
                crate::mcp_keys::forget_server(&self.shared, &name).await;
                result
            }
            ConnectorsRequest::SetServerOff { name, off } => {
                let params = json!({"name": name, "source": "server", "disabled": off});
                self.server_change(origin, "mcp_catalog/toggle", params)
                    .await?
            }
            request @ (ConnectorsRequest::AddKeyServer { .. }
            | ConnectorsRequest::AllowServerKey { .. }) => self.key_server(origin, request).await?,
        };
        Ok(CommandResult::Connectors { result })
    }

    /// `addKeyServer` and `allowServerKey` (`mcp_keys`), then every open session re-reads its servers.
    async fn key_server(
        &self,
        origin: Origin,
        request: ConnectorsRequest,
    ) -> Result<ConnectorsResult, CoreError> {
        match request {
            ConnectorsRequest::AddKeyServer {
                url,
                name,
                secret_id,
                new_secret,
                header,
            } => {
                let request = crate::mcp_keys::AddKeyServer {
                    url,
                    name,
                    secret_id,
                    new_secret,
                    header,
                };
                let name = crate::mcp_keys::add(&self.shared, origin, request).await?;
                self.shared.connectors.mark_removed(&name, false);
                self.converge(Converge::Servers).await;
                Ok(ConnectorsResult::ServerAdded {
                    name,
                    created: true,
                    view: self.connectors_view(origin, None).await,
                })
            }
            ConnectorsRequest::AllowServerKey { name } => {
                checked_name(&name)?;
                crate::mcp_keys::allow(&self.shared, origin, &name).await?;
                self.converge(Converge::Servers).await;
                Ok(self.view_result(origin).await)
            }
            other => Err(invalid(format!("not a key server request: {other:?}"))),
        }
    }

    async fn view_result(&self, origin: Origin) -> ConnectorsResult {
        ConnectorsResult::View {
            view: self.connectors_view(origin, None).await,
        }
    }

    /// A server change (sign out, remove, toggle) on the lobby, then every open session converges.
    async fn server_change(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
    ) -> Result<ConnectorsResult, CoreError> {
        if let Some(name) = params.get("name").and_then(Value::as_str) {
            checked_name(name)?;
        }
        let removed = (method == "mcp_catalog/remove")
            .then(|| {
                params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten();
        let reply = self.forward(origin, method, params, None).await?;
        if let Some(name) =
            removed.filter(|_| reply.result.get("removed") == Some(&Value::Bool(true)))
        {
            self.shared.connectors.mark_removed(&name, true);
            crate::ask::grants::connector_removed(&self.shared);
        }
        self.converge(Converge::Servers).await;
        Ok(self.view_result(origin).await)
    }

    /// A call that needs a confirmation token: from the app's window a trusted sign-in start (the gate
    /// mints the token without a dialog), otherwise the native dialog.
    async fn signed_call(
        &self,
        origin: Origin,
        method: &str,
        params: Value,
    ) -> Result<Value, CoreError> {
        let token = self.token_for(origin, method, &params).await?;
        let reply = self
            .forward(origin, method, params, token.map(|t| t.as_str().to_owned()))
            .await?;
        Ok(reply.result)
    }

    /// The token for `method`: none needed from the window for a trusted method (`forward` mints it), else
    /// the native dialog's.
    pub(crate) async fn token_for(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
    ) -> Result<Option<ConfirmationToken>, CoreError> {
        let window_trusted = origin == Origin::Ui
            && self.shared.config.trusted_ui_shell
            && TRUSTED_SURFACE_METHODS.contains(&method);
        if window_trusted {
            return Ok(None);
        }
        let requester = crate::command::requester(origin);
        match self
            .shared
            .gate
            .request_confirmation_as(method, params, requester)
            .await
        {
            Ok(token) => Ok(Some(token)),
            Err(GateError::ConfirmationNotApplicable { .. }) => Ok(None),
            Err(error) => Err(CoreError::from_gate(method, &error)),
        }
    }

    /// `addServer`: the address rule first (no dialog for an address the gate would refuse), then the
    /// native dialog from every origin, then `mcp_catalog/add` on the lobby.
    async fn add_server(
        &self,
        origin: Origin,
        url: String,
        name: Option<String>,
    ) -> Result<ConnectorsResult, CoreError> {
        // Only the canonical form is sent: the address Vibe parses must be the one checked.
        let url =
            vw_gate::check_mcp_server_url(&url).map_err(|refusal| invalid(refusal.to_string()))?;
        if let Some(name) = &name {
            checked_name(name)?;
        }
        let mut params = Map::new();
        params.insert("url".to_owned(), Value::String(url));
        if let Some(name) = name {
            params.insert("name".to_owned(), Value::String(name));
        }
        let params = Value::Object(params);
        let token = self.token_for(origin, "mcp_catalog/add", &params).await?;
        let reply = self
            .forward(
                origin,
                "mcp_catalog/add",
                params,
                token.map(|t| t.as_str().to_owned()),
            )
            .await?;
        let name = reply
            .result
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        self.shared.connectors.mark_removed(&name, false);
        self.converge(Converge::Servers).await;
        let created = reply.result.get("created").and_then(Value::as_bool) == Some(true);
        info!(created, "connectors: MCP server added");
        Ok(ConnectorsResult::ServerAdded {
            name,
            created,
            view: self.connectors_view(origin, None).await,
        })
    }

    /// `signInServer`: checks and confirms now, then runs Vibe's blocking login in an owned task.
    async fn start_server_sign_in(
        &self,
        origin: Origin,
        name: String,
    ) -> Result<ConnectorsResult, CoreError> {
        if let Some(running) = self.shared.connectors.running() {
            let mut error = CoreError::new(
                ErrorCode::Denied,
                format!("the sign-in of \"{running}\" is still running: finish or stop it first"),
            );
            error.server_code = Some("signInRunning".to_owned());
            return Err(error);
        }
        let params = json!({"name": name});
        // Validated and confirmed before the task starts, so a refusal reaches the caller.
        self.shared
            .gate
            .check("mcp_catalog/login", &params, None)
            .or_else(|e| match e {
                GateError::ConfirmationRequired { .. } => Ok(()),
                other => Err(other),
            })
            .map_err(|e| CoreError::from_gate("mcp_catalog/login", &e))?;
        let token = self.token_for(origin, "mcp_catalog/login", &params).await?;
        let weak = Arc::downgrade(&self.shared);
        let task_name = name.clone();
        let task = tokio::spawn(async move {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let core = Core { shared };
            let outcome = core
                .forward(
                    origin,
                    "mcp_catalog/login",
                    params,
                    token.map(|t| t.as_str().to_owned()),
                )
                .await;
            let (outcome, message) = match outcome {
                Ok(_) => ("signedIn", None),
                Err(error) => {
                    warn!(
                        code = error.code.as_str(),
                        "connectors: server sign-in failed"
                    );
                    ("failed", Some(error.message))
                }
            };
            if outcome == "signedIn" {
                core.converge(Converge::Servers).await;
            }
            core.shared.connectors.slot().take();
            core.shared.emit(CoreEvent::ConnectorSignIn {
                name: task_name,
                outcome: outcome.to_owned(),
                message,
            });
        });
        *self.shared.connectors.slot() = Some((name.clone(), task));
        Ok(ConnectorsResult::ServerSignInStarted { name })
    }

    /// Loads the account's catalog on the lobby (`connector_catalog/refresh`). Returns Vibe's message when
    /// it failed.
    async fn refresh_catalog(&self, origin: Origin) -> Option<String> {
        self.shared
            .connectors
            .catalog_tried
            .store(true, Ordering::Relaxed);
        match self
            .forward(origin, "connector_catalog/refresh", json!({}), None)
            .await
        {
            Ok(_) => None,
            Err(error) => {
                warn!(
                    code = error.code.as_str(),
                    "connectors: catalog refresh failed"
                );
                Some(error.message)
            }
        }
    }

    /// Asks every session with an open channel to pick up a change made on the lobby.
    async fn converge(&self, what: Converge) {
        converge_shared(&self.shared, what).await;
    }

    /// The view: the lobby catalog (loaded once when Vibe has not), Vibe Work's configured servers, and
    /// the live state of `session_id` when that session has an open channel.
    async fn connectors_view(&self, origin: Origin, session_id: Option<String>) -> ConnectorsView {
        let mut catalog_error = None;
        let mut read = self.lobby_read(origin).await;
        let not_loaded = read
            .as_ref()
            .is_ok_and(|r| catalog_load(r) == crate::connectors_view::CatalogLoad::NotLoaded);
        // A read must not start a provider request while the selected vault cannot supply a key.
        // Leave catalog_tried clear so the first read after unlock or setup can load the catalog.
        if not_loaded && !self.shared.connectors.catalog_tried.load(Ordering::Relaxed) {
            let key_ready = match &self.shared.config.keys {
                Some(keys) => keys
                    .status()
                    .await
                    .is_ok_and(|status| matches!(status.key, KeyPresence::Ready { .. })),
                None => true, // Development runs without a key service keep their existing behavior.
            };
            if key_ready {
                catalog_error = self.refresh_catalog(origin).await;
                read = self.lobby_read(origin).await;
            }
        }
        let read = read.unwrap_or_else(|message| {
            catalog_error.get_or_insert(message);
            Value::Null
        });
        let mut list = connectors(&read);
        let mut servers = configured_servers(&self.shared.instructions.own_home);
        // key servers show their key's name, header and grant, and the address the key goes to.
        crate::mcp_keys::merge_views(&self.shared, &mut servers).await;
        let open = session_id.filter(|id| self.shared.channels.has_channel(id));
        if let Some(id) = &open {
            let params = json!({"sessionId": id});
            match self
                .shared
                .gate
                .call("connector_catalog/read", params.clone(), None)
                .await
            {
                Ok(reply) => merge_session_connectors(&mut list, &reply.result),
                Err(error) => {
                    warn!(%error, "connectors: the session's connector state was not read");
                }
            }
            match self
                .shared
                .gate
                .call("mcp_catalog/read", params.clone(), None)
                .await
            {
                Ok(reply) => merge_session_servers(&mut servers, &reply.result),
                Err(error) => warn!(%error, "connectors: the session's servers were not read"),
            }
            match self
                .shared
                .gate
                .call("plugin_catalog/read", params, None)
                .await
            {
                Ok(reply) => merge_plugin_sources(&mut servers, &reply.result),
                Err(error) => {
                    warn!(%error, "connectors: the pinned plugins' origins were not read");
                }
            }
        }
        if let Some(id) = &open
            && self
                .shared
                .agent_tools
                .as_ref()
                .is_some_and(|tools| tools.table.has_session(id))
        {
            crate::connectors_project::mark_injected_server(&mut servers);
        }
        ConnectorsView {
            catalog: catalog_load(&read),
            catalog_error,
            private_connectors_hidden: read.get("privateConnectorsHidden").and_then(Value::as_bool),
            connectors: list,
            servers,
            manage_url: read
                .get("manageUrl")
                .and_then(Value::as_str)
                .and_then(crate::connectors_project::manage_link),
            session_id: open,
            signing_in: self.shared.connectors.running(),
        }
    }

    async fn lobby_read(&self, origin: Origin) -> Result<Value, String> {
        self.forward(origin, "connector_catalog/read", json!({}), None)
            .await
            .map(|reply| reply.result)
            .map_err(|error| error.message)
    }
}

/// Asks every session with an open channel to pick up a change made on the lobby.
async fn converge_shared(shared: &Shared, what: Converge) {
    let method = match what {
        Converge::Connectors => "connector_catalog/refresh",
        Converge::Servers => "mcp_catalog/refresh",
    };
    for open in shared.channels.open_channels() {
        let params = json!({"sessionId": open.session_id});
        if let Err(error) = shared.gate.call(method, params, None).await {
            warn!(method, %error, "connectors: a session did not take the change");
        }
    }
}

/// Lets every open session re-read its MCP servers (after a key grant was revoked elsewhere).
pub(crate) async fn refresh_servers(shared: &Shared) {
    converge_shared(shared, Converge::Servers).await;
}

/// Aborts a running server sign-in (core shutdown), and removes the MCP forwarder's resolver, so no
/// request is forwarded with a key once the core shuts down.
pub(crate) fn abort_sign_in(shared: &Shared) {
    crate::mcp_keys::uninstall(shared);
    if let Some((_, task)) = shared.connectors.slot().take() {
        task.abort();
    }
}

#[cfg(test)]
mod fx18_tests {
    use super::ConnectorsState;

    #[test]
    fn removed_server_name_matches_only_its_tools_until_readded() {
        let state = ConnectorsState::default();
        state.mark_removed("notes", true);
        assert!(state.removed_tool("mcp_notes.list_notes"));
        assert!(!state.removed_tool("mcp_notes_extra.list_notes"));
        state.mark_removed("notes", false);
        assert!(!state.removed_tool("mcp_notes.list_notes"));
    }
}
