//! Re-pin a chat, then rebind its host hooks before admitting another turn.
//! Depends on channels, recovery and `on_demand`; called by marketplace, commands and stale-pin repair.
//! The per-chat lock covers reload, close and verified attach. Pending work shares Chrome's reopen.
//! A chat this core started this run and never sent work only re-pins (no reattach). A failed re-pin stays
//! pending, so every later send of that chat retries it first and is refused while it fails.
//! Withheld MCP discovery after a completed re-pin is reported without retrying the completed work.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use vw_platform::sync::LockRecover;

use serde_json::{Value, json};
use tracing::warn;

use crate::command::Origin;
use crate::core::Core;
use crate::developer_permissions::DeveloperPermissions;
use crate::error::{CoreError, ErrorCode};
use crate::event::{AlertKind, CoreEvent};
use crate::mcp_keys_config::ConfigEditError;

pub(crate) const MCP_DISCOVERY_WITHHELD: &str = "pluginMcpDiscoveryWithheld";

async fn discovery_guard<'a>(
    permissions: &'a DeveloperPermissions,
    home: &Path,
    guard_installed: impl FnOnce() -> bool,
) -> Result<Option<tokio::sync::MutexGuard<'a, ()>>, ConfigEditError> {
    let held = permissions.mutation_lock().await;
    if crate::mcp_refresh::blocked_command(home, permissions.enabled(), guard_installed())? {
        return Ok(None);
    }
    Ok(Some(held))
}

#[derive(Default)]
struct Inner {
    locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    pending: HashSet<String>,
    /// Also covers cancellation/timeouts: no subsequent send may reuse a hookless channel.
    reopen: HashSet<String>,
    /// Chats this core sent (or may have sent) work this run: a turn, a queue item, a shell command or context.
    admitted: HashSet<String>,
    /// Chats this core started (`session/start` answered) this run.
    created: HashSet<String>,
}

#[derive(Default)]
pub(crate) struct Refresh {
    inner: Mutex<Inner>,
}

impl Refresh {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock_or_recover()
    }

    pub(crate) async fn lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.inner().locks.entry(id.to_owned()).or_default().clone();
        lock.lock_owned().await
    }

    pub(crate) fn request(&self, id: &str) {
        self.inner().pending.insert(id.to_owned());
    }

    pub(crate) fn pending(&self, id: &str) -> bool {
        self.inner().pending.contains(id)
    }

    pub(crate) fn take(&self, id: &str) -> bool {
        self.inner().pending.remove(id)
    }

    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let mut inner = self.inner();
        if inner.pending.remove(old) {
            inner.pending.insert(new.to_owned());
        }
        if inner.reopen.remove(old) {
            inner.reopen.insert(new.to_owned());
        }
        if inner.admitted.remove(old) {
            inner.admitted.insert(new.to_owned());
        }
        // A handed-off conversation has run: never "unstarted".
        inner.created.remove(old);
    }

    pub(crate) fn forget(&self, id: &str) {
        let mut inner = self.inner();
        inner.pending.remove(id);
        inner.reopen.remove(id);
        inner.admitted.remove(id);
        inner.created.remove(id);
    }

    /// Marks `params`' chat started BEFORE work is sent (the caller holds the chat lock). Dropping the guard (an
    /// answer, no answer, a cancelled caller) keeps the mark; only [`Admission::refused_by`] a refusal from before
    /// the send undoes the mark this call made. A chat that becomes started while a re-pin is still pending (it could
    /// not run first) is reattached before its next send: its runtime may now hold the stale plugins without hooks.
    pub(crate) fn admission(&self, method: &str, params: &Value) -> Admission<'_> {
        let id = params["sessionId"]
            .as_str()
            .filter(|_| builds_runtime(method))
            .filter(|id| {
                let mut inner = self.inner();
                let first = inner.admitted.insert((*id).to_owned());
                if first && inner.pending.contains(*id) {
                    inner.reopen.insert((*id).to_owned());
                }
                first
            })
            .map(str::to_owned);
        Admission { refresh: self, id }
    }

    pub(crate) fn created(&self, id: &str) {
        self.inner().created.insert(id.to_owned());
    }

    /// This core started the chat this run (`session/start`), whether or not it has sent it work since.
    pub(crate) fn created_here(&self, id: &str) -> bool {
        self.inner().created.contains(id)
    }

    /// Only in-memory facts may skip the guard reattach: this core started the chat this run, never sent it work,
    /// and the store shows no turn. Any doubt (another start path, a restart, an unclear send) means started.
    pub(crate) fn unstarted(&self, shared: &crate::core::Shared, id: &str) -> bool {
        let known = {
            let inner = self.inner();
            inner.created.contains(id) && !inner.admitted.contains(id)
        };
        known && crate::recovery::never_ran(shared, id)
    }

    pub(crate) fn needs_reopen(&self, id: &str) -> bool {
        self.inner().reopen.contains(id)
    }
}

/// See [`Refresh::admission`].
pub(crate) struct Admission<'a> {
    refresh: &'a Refresh,
    id: Option<String>,
}

impl Admission<'_> {
    /// Undoes the mark only when the request never left the core: the gate refused it before sending, or it could not
    /// be encoded. Every other outcome keeps it, Vibe's own error answer included (Vibe may have built the runtime
    /// before it failed, e.g. a subagent setup error after `promote`), as do a timeout, a dead server, an unreadable
    /// reply and a local transport failure.
    pub(crate) fn refused_by(mut self, error: &vw_gate::GateError) {
        let unsent = !matches!(error, vw_gate::GateError::Transport(transport)
            if !matches!(transport, vw_rpc::RpcError::Encode { .. }));
        if unsent && let Some(id) = self.id.take() {
            self.refresh.inner().admitted.remove(&id);
        }
    }
}

/// The calls after which Vibe has built a never-run chat's runtime (vibe 3.0.x `_session.py`: every caller of
/// `_runtime_for_turn_locked`: start, enqueue, queue replace, context inject, and the shell command's host effect).
pub(crate) fn builds_runtime(method: &str) -> bool {
    crate::changes::starts_work(method)
        || matches!(
            method,
            "session/turn/queue/replace" | "session/context/inject"
        )
}

pub(crate) fn reopens(method: &str) -> bool {
    matches!(
        method,
        "session/resume" | "session/continue" | "session/fork"
    )
}

impl Core {
    /// Serializes calls for one chat, including any pending refresh before new work starts.
    pub(crate) async fn plugin_refresh_lock(
        &self,
        method: &str,
        params: &Value,
        continuation: Option<u64>,
    ) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>, CoreError> {
        if method == "plugin/reload" {
            self.shared
                .gate
                .check(method, params, None)
                .map_err(|error| CoreError::from_gate(method, &error))?;
        }
        let Some(id) = params["sessionId"]
            .as_str()
            .or_else(|| params["sourceSessionId"].as_str())
        else {
            return Ok(None);
        };
        let id = self.shared.channels.canonical(id);
        let held = self.shared.plugin_refresh.lock(&id).await;
        if continuation.is_none() && matches!(method, "turn/start" | "session/resume") {
            crate::interrupted_send::prepare(self, &id).await?;
        }
        if self.shared.plugin_refresh.needs_reopen(&id)
            && (crate::routing::attaches_session(method, params).is_some()
                || reopens(method)
                || method == "plugin/reload")
        {
            self.reopen_tools(&id).await?;
        }
        // Explicit deferred closure (instructions/key refresh) also applies to a viewed chat. The idle
        // reaper preserves viewed channels, so finish the refresh here before admitting its next work.
        if builds_runtime(method)
            && self.shared.channels.closing_when_quiet(&id)
            && crate::recovery::quiet(&self.shared, &id)
        {
            Box::pin(self.shared.channels.close(&id)).await;
        }
        // Every call that can build a never-run chat's runtime first applies a pending re-pin; a failed re-pin keeps
        // the change pending and refuses this work (`apply_pending`).
        if builds_runtime(method) && crate::on_demand::ready(&self.shared, &id) {
            crate::on_demand::apply_pending(&self.shared, &id)
                .await
                .map_err(|mut error| {
                    error.message = format!("Not sent: {}", error.message);
                    error
                })?;
        }
        Ok(Some(held))
    }

    /// A cancelled client cannot interrupt the reload/reopen worker. Busy chats return pending.
    pub(crate) async fn refresh_plugins(&self, id: &str) -> Result<bool, CoreError> {
        crate::sessions::snapshot(&self.shared, id)?;
        self.shared.plugin_refresh.request(id);
        let (shared, id) = (self.shared.clone(), id.to_owned());
        tokio::spawn(async move {
            crate::on_demand::switch(shared.clone(), id.clone(), false).await?;
            Ok(shared.plugin_refresh.pending(&id))
        })
        .await
        .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))?
    }

    /// Re-pin only a chat attached by this call, under its source lock or a new fork's own lock.
    pub(crate) async fn plugins_after_open(
        &self,
        origin: Origin,
        restore: Option<crate::restore::Restoring>,
        method: &str,
        params: &Value,
        result: &Value,
    ) -> Result<(), CoreError> {
        if let Some(restore) = restore {
            self.restore_profile(origin, restore).await?;
        }
        if method == "session/start"
            && let Some(id) = crate::routing::result_session(result)
        {
            self.shared.plugin_refresh.created(id);
            crate::unsaved_stores::started(&self.shared, id);
        }
        if reopens(method)
            && let Some(id) = crate::routing::result_session(result)
        {
            let source = params["sessionId"]
                .as_str()
                .or_else(|| params["sourceSessionId"].as_str())
                .map(|source| self.shared.channels.canonical(source));
            let _held = if source.as_deref() == Some(id) {
                None
            } else {
                Some(self.shared.plugin_refresh.lock(id).await)
            };
            self.refresh_removed_plugins(id).await?;
        }
        Ok(())
    }

    /// Called under the same chat lock as ordinary sends and resumes.
    pub(crate) async fn refresh_removed_plugins(&self, id: &str) -> Result<(), CoreError> {
        if !self.shared.channels.has_channel(id) {
            return Ok(());
        }
        let switched = self.native_plugin_switches_changed(id).await?;
        if self.shared.channels.has_removed_plugins(id).await || switched {
            self.refresh_plugins_held(id).await?;
        }
        Ok(())
    }

    /// The sole internal plugin/reload sender. Vibe rebinds the old lock on reload failure.
    pub(crate) async fn refresh_plugins_held(&self, id: &str) -> Result<(), CoreError> {
        crate::project_mcp::admit(&self.shared, "plugin/reload", &json!({"sessionId": id})).await?;
        // A chat that ran is reattached after the reload, which binds Vibe Work's host hooks to it again. A chat
        // that never ran has no runtime yet: Vibe keeps the re-pin as its pending binding and builds the runtime at
        // its first turn from the session's own hook bindings (`_session.py::_rewrite_plugins`, `_host.py` `promote`).
        // Reattaching it would discard that binding, so it only re-pins, whatever the reload's outcome.
        let unstarted = self.shared.plugin_refresh.unstarted(&self.shared, id);
        if !unstarted {
            self.shared
                .plugin_refresh
                .inner()
                .reopen
                .insert(id.to_owned());
        }
        if let Err(error) = self
            .shared
            .channels
            .request("plugin/reload", json!({"sessionId": id}))
            .await
        {
            let rejected = matches!(
                &error,
                vw_rpc::RpcError::Remote { .. } | vw_rpc::RpcError::Local { .. }
            );
            if rejected {
                self.shared.plugin_refresh.inner().reopen.remove(id);
            }
            // A lost reply is not a rejection: keep the reopen requirement until hooks are rebound.
            warn!(session_id = id, %error, "plugins: reload did not finish");
            let message = if rejected {
                "This chat could not refresh its plugins and still has its previous plugins."
            } else {
                "This chat could not finish refreshing its plugins; send a message to reopen it."
            };
            self.shared.emit(CoreEvent::Alert {
                session_id: Some(id.to_owned()),
                kind: AlertKind::PluginReloadFailed,
                message: message.to_owned(),
            });
            let mut failure = CoreError::new(
                if rejected {
                    ErrorCode::ServerError
                } else {
                    ErrorCode::ServerUnavailable
                },
                message,
            );
            failure.server_code = Some(
                if rejected {
                    "pluginReloadRejected"
                } else {
                    "pluginReloadUncertain"
                }
                .to_owned(),
            );
            return Err(failure);
        }
        // Vibe can acknowledge a reload while parking it. Verify native OFF before
        // reopening (which itself inspects pins), so a stale set cannot recurse or be admitted.
        if self.shared.channels.has_removed_plugins(id).await
            || self.native_plugin_switches_changed(id).await?
        {
            self.shared.plugin_refresh.request(id);
            return Err(CoreError::new(
                ErrorCode::ServerError,
                "Plugin refresh did not finish: a removed or disabled plugin is still attached. Reopen the chat before sending.",
            ));
        }
        if unstarted {
            return Ok(());
        }
        self.reopen_tools(id).await?;
        self.discover_plugin_mcp_with_children(id).await?;
        Ok(())
    }

    /// After the reattach of a chat that has subagents, `mcp_catalog/refresh` makes Vibe discover the MCP servers a
    /// just-turned-on plugin brings, so their tools are there for its later turns (a chat without subagents gets
    /// them from the reattach itself). A failure leaves the chat usable with its other
    /// tools and is only logged. Disabled command rows are safe only when the running server has the guard.
    async fn discover_plugin_mcp_with_children(&self, id: &str) -> Result<(), CoreError> {
        let has_children = self.shared.store.view(id).is_some_and(|view| {
            view.state["childSessions"]
                .as_array()
                .is_some_and(|children| !children.is_empty())
        });
        if !has_children {
            return Ok(());
        }
        // Keep the OFF transition from landing between the check and Vibe's
        // forced discovery. Disabled stdio is safe only when this server runs through the adapter.
        let _mutation = match discovery_guard(
            &self.shared.developer_permissions,
            &self.shared.instructions.own_home,
            || crate::mcp_refresh::guard_installed(&self.shared),
        )
        .await
        {
            Ok(Some(held)) => held,
            Ok(None) => {
                if self.shared.developer_permissions.enabled() {
                    let message =
                        vw_vocabulary::texts::COMMAND_SERVER_DISABLED_PLUGIN_REFRESH_BLOCKED;
                    self.shared.emit(CoreEvent::Alert {
                        session_id: Some(id.to_owned()),
                        kind: AlertKind::ToolsUnavailable,
                        message: message.to_owned(),
                    });
                    let mut error = CoreError::new(ErrorCode::Denied, message);
                    error.server_code = Some(MCP_DISCOVERY_WITHHELD.to_owned());
                    return Err(error);
                }
                crate::command_pause::notify_internal_refresh_withheld(
                    &self.shared,
                    Some(id),
                    crate::developer_permissions::RefreshWithheldCause::PausedCommandServer,
                );
                return Ok(());
            }
            Err(error) => {
                warn!(session_id = id, %error, "plugins: MCP discovery withheld while command servers cannot be checked");
                crate::command_pause::notify_internal_refresh_withheld(
                    &self.shared,
                    Some(id),
                    crate::developer_permissions::RefreshWithheldCause::InspectFailed,
                );
                return Ok(());
            }
        };
        if let Err(error) = crate::project_mcp::admit(
            &self.shared,
            "mcp_catalog/refresh",
            &json!({"sessionId": id}),
        )
        .await
        {
            warn!(session_id = id, reason = %error.message, "plugins: project MCP refresh refused");
            self.shared.emit(CoreEvent::Alert {
                session_id: Some(id.to_owned()),
                kind: AlertKind::ToolsUnavailable,
                message: error.message,
            });
            return Ok(());
        }
        if let Err(error) = self
            .shared
            .channels
            .request("mcp_catalog/refresh", json!({"sessionId": id}))
            .await
        {
            warn!(session_id = id, %error, "plugins: MCP discovery after the reopen did not finish");
        }
        Ok(())
    }

    /// Close and attach are one bounded operation, shared with the on-demand tool switch.
    pub(crate) async fn reopen_tools(&self, id: &str) -> Result<(), CoreError> {
        let bound = self.shared.config.reopen_timeout;
        let reopened = tokio::time::timeout(bound, async {
            self.shared.channels.close(id).await;
            self.attach_session(Origin::Ui, id).await
        })
        .await
        .unwrap_or_else(|_| {
            Err(CoreError::new(
                ErrorCode::Timeout,
                format!("no answer within {}s", bound.as_secs()),
            ))
        });
        if let Err(error) = &reopened {
            if self.shared.interrupted_send.pending(id).is_some() {
                self.shared.channels.close(id).await;
            }
            warn!(session_id = id, %error, "the chat did not reopen with its new tools");
            if error.code != ErrorCode::AccessModeUnverified {
                let title = self
                    .shared
                    .names
                    .title_of(&self.shared.store, id)
                    .unwrap_or_else(|| {
                        vw_vocabulary::text("untitledSession")
                            .unwrap_or_default()
                            .to_owned()
                    });
                self.shared.emit(CoreEvent::Alert {
                    session_id: Some(id.to_owned()), kind: AlertKind::ReopenFailed,
                    message: format!("“{title}” could not reopen with its new tools. Send a message to try again."),
                });
            }
        } else {
            self.shared.plugin_refresh.inner().reopen.remove(id);
        }
        reopened
    }
}

#[cfg(test)]
mod tests {
    use super::{Refresh, discovery_guard};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use vw_gate::GateError;
    use vw_rpc::RpcError;

    fn remote() -> GateError {
        GateError::Transport(RpcError::Remote {
            method: "turn/start".into(),
            error: serde_json::from_value(json!({"code":"conflict","message":"no"})).unwrap(),
        })
    }

    #[tokio::test]
    async fn internal_discovery_admits_disabled_row_in_both_switch_states() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true}]\n",
        )
        .unwrap();
        let permissions = crate::developer_permissions::DeveloperPermissions::new(None);
        for enabled in [false, true] {
            permissions.set_for_test(enabled);
            assert!(
                discovery_guard(&permissions, dir.path(), || true)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn internal_discovery_keeps_clean_on_configs_admitted() {
        let dir = tempfile::tempdir().unwrap();
        let permissions = crate::developer_permissions::DeveloperPermissions::new(None);
        permissions.set_for_test(true);
        for text in [
            "",
            "mcp_servers=[{name='x',transport='stdio',command='echo'}]",
            "mcp_servers=[{name='h',transport='http',disabled=true}]",
        ] {
            std::fs::write(dir.path().join("config.toml"), text).unwrap();
            assert!(
                discovery_guard(&permissions, dir.path(), || true)
                    .await
                    .unwrap()
                    .is_some(),
                "{text}"
            );
        }
    }

    #[tokio::test]
    async fn internal_discovery_refreshes_without_refusal_while_on() {
        use crate::refresh_notice_tests::{
            attach_mock_channel, count_server_refreshes, drain_events, start_with_keys,
            tools_alerts,
        };
        let core = start_with_keys(
            "addrefresh-plugin-notice",
            None,
            true,
            "mcp_servers=[{name='y',transport='stdio',command='echo',disabled=true}]",
        )
        .await;
        let calls = Arc::default();
        let _peer = attach_mock_channel(&core, "s1", 1, Arc::clone(&calls));
        let _ = core.shared.store.load(json!({
            "session": {"id": "s1", "cwd": core.shared.default_cwd, "status": {"type": "idle"}},
            "entries": [], "turns": [], "childSessions": [{"id": "child", "status": {"type": "idle"}}]
        }), vw_session::LoadKind::New).unwrap();
        let mut sub = core.subscribe(None);
        let _ = drain_events(&mut sub).await;
        core.discover_plugin_mcp_with_children("s1").await.unwrap();
        assert_eq!(count_server_refreshes(&calls), 1);
        assert_eq!(
            tools_alerts(&drain_events(&mut sub).await),
            Vec::<(Option<String>, String)>::new()
        );
        core.shutdown().await;
    }

    #[tokio::test]
    async fn internal_discovery_rechecks_off_with_enabled_row_after_waiting_for_the_mutation_lock()
    {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        std::fs::write(
            home.join("config.toml"),
            "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = false }]\n",
        )
        .unwrap();
        let permissions = Arc::new(crate::developer_permissions::DeveloperPermissions::new(
            None,
        ));
        permissions.set_for_test(true);
        let changing = permissions.mutation_lock().await;
        let waiting = permissions.clone();
        let task = tokio::spawn(async move {
            discovery_guard(&waiting, &home, || true)
                .await
                .unwrap()
                .is_some()
        });
        tokio::task::yield_now().await;
        permissions.set_for_test(false);
        drop(changing);
        assert!(
            !task.await.unwrap(),
            "an unsafe command refresh would have been sent after OFF"
        );
    }

    async fn drain_tools_alerts(
        sub: &mut crate::CoreSubscription,
    ) -> Vec<(Option<String>, String)> {
        let mut alerts = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), sub.next()).await
        {
            if let crate::CoreEvent::Alert {
                session_id,
                kind: crate::AlertKind::ToolsUnavailable,
                message,
            } = event
            {
                alerts.push((session_id, message));
            }
        }
        alerts
    }

    #[tokio::test]
    async fn unsafe_plugin_discovery_while_off_emits_deduplicated_tools_alert() {
        let core = crate::offline_core::start_with("plugin-refresh-withheld", |config, dir| {
            let app_data = dir.join("app-data");
            let vibe_home = dir.join("vibe-home");
            std::fs::create_dir_all(&app_data).unwrap();
            std::fs::create_dir_all(&vibe_home).unwrap();
            std::fs::write(
                app_data.join("settings.json"),
                r#"{"developerPermissions":false}"#,
            )
            .unwrap();
            std::fs::write(
                vibe_home.join("config.toml"),
                "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = true }]\n",
            )
            .unwrap();
            config.secret_scan_data = Some(app_data);
        })
        .await;
        // An enabled row introduced behind the app's back while OFF must still block discovery.
        let path = core.shared.instructions.own_home.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, text.replace("disabled = true", "disabled = false")).unwrap();
        let _ = core
            .shared
            .store
            .load(
                json!({
                    "session": {"id": "s1", "cwd": "/project", "status": {"type": "idle"}},
                    "entries": [],
                    "turns": [],
                    "childSessions": [{"id": "c1", "status": {"type": "idle"}}]
                }),
                vw_session::LoadKind::New,
            )
            .unwrap();
        let mut sub = core.subscribe(None);
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), sub.next()).await {}

        core.discover_plugin_mcp_with_children("s1").await.unwrap();
        let first = drain_tools_alerts(&mut sub).await;
        assert_eq!(
            first.len(),
            1,
            "expected one plugin discovery withheld alert: {first:?}"
        );
        assert_eq!(first[0].0.as_deref(), Some("s1"));
        assert_eq!(
            first[0].1,
            vw_vocabulary::texts::COMMAND_SERVER_INTERNAL_REFRESH_BLOCKED,
            "{first:?}"
        );

        core.discover_plugin_mcp_with_children("s1").await.unwrap();
        let second = drain_tools_alerts(&mut sub).await;
        assert_eq!(
            second,
            Vec::<(Option<String>, String)>::new(),
            "second withheld discovery must be deduplicated"
        );
        // Cross-path dedupe: an internal converge for the same paused cause in the same OFF period
        // must also stay quiet.
        crate::connectors::refresh_servers(&core.shared).await;
        let converge = drain_tools_alerts(&mut sub).await;
        assert_eq!(
            converge,
            Vec::<(Option<String>, String)>::new(),
            "converge after plugin discovery in same OFF period must be deduplicated"
        );
        core.shutdown().await;
    }

    #[test]
    fn work_is_started_before_it_is_sent_and_only_a_refusal_before_sending_undoes_it() {
        let refresh = Refresh::default();
        let params = json!({"sessionId": "s"});
        let admitted = |r: &Refresh| r.inner().admitted.contains("s");
        // A read never marks.
        drop(refresh.admission("session/read", &params));
        assert!(!admitted(&refresh));
        // Our gate refused it before sending: nothing started.
        refresh
            .admission("turn/start", &params)
            .refused_by(&GateError::Denied {
                method: "turn/start".into(),
                reason: vw_gate::DenyReason::Unclassified,
            });
        assert!(!admitted(&refresh));
        // Vibe's own error answer: Vibe may have built the runtime first, so the chat stays started.
        refresh
            .admission("turn/start", &params)
            .refused_by(&remote());
        assert!(admitted(&refresh));
        refresh.forget("s");
        assert!(!admitted(&refresh));
        // No answer (timeout): may have started.
        refresh
            .admission("turn/start", &params)
            .refused_by(&GateError::Transport(RpcError::Timeout {
                method: "turn/start".into(),
                id: 3,
                after: Duration::from_secs(30),
            }));
        assert!(admitted(&refresh));
        refresh.forget("s");
        // A cancelled caller drops the guard: started.
        drop(refresh.admission("session/turn/enqueue", &params));
        assert!(admitted(&refresh));
        // A later refusal does not undo an earlier mark.
        refresh
            .admission("turn/start", &params)
            .refused_by(&GateError::Denied {
                method: "turn/start".into(),
                reason: vw_gate::DenyReason::Unclassified,
            });
        assert!(admitted(&refresh));
        // Context injection and a queue replace also build Vibe's runtime.
        for method in ["session/context/inject", "session/turn/queue/replace"] {
            refresh.forget("s");
            drop(refresh.admission(method, &params));
            assert!(admitted(&refresh), "{method}");
        }
        // Started while a re-pin is still pending: reattached before the next send.
        refresh.forget("s");
        refresh.request("s");
        drop(refresh.admission("turn/start", &params));
        assert!(refresh.needs_reopen("s"));
    }
}
