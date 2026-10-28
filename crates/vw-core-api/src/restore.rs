//! Putting a resumed session back in its access mode, failing closed.
//!
//! Why: the core keeps each session's access mode and applies it itself on every reopen (`access.rs`).
//! Every resume names the resolved profile and the session is then switched
//! (`session/agent/update`) to the profile it should run in, before any of its commands is sent.
//!
//! Which profile, in order: the one this core last saw for the session (`access.rs`), else Vibe's own
//! record of it (`PublicSession.agent`, from the store or a `session/read` before the resume; it may
//! be empty), else `ask`. There is no "nothing to restore" path.
//!
//! Depends on: `core` (`Core`, `Shared`), `access`, `sessions`, `channels`.
//! Called by: `recovery::open_session` and `dispatch::forward` (a client's resume).
//! Invariants:
//! - While a restore runs the session counts as having no channel (`Channels::begin_restore`): a
//!   concurrent command waits on the per-session restore lock and uses the verified channel.
//! - The switch counts only when Vibe's answer reports the requested profile as active. Otherwise the
//!   session's channel is closed (no usable channel is ever left in an unverified mode), the command
//!   is refused with `accessModeUnverified`, an alert says so, and the next command tries again.
//! - Full access (`auto-approve`) in this app's own record of the session (`access.rs`: the user confirmed it
//!   for this session, in this run or an earlier one; never Vibe's session file, which the agent can write)
//!   is put back: the resume names `ask` (a resume may not carry Full
//!   access without a dialog) and the switch then selects `auto-approve` with a trusted token for that exact
//!   call (`Gate::request_trusted_confirmation`), so no new dialog appears (the mode the
//!   user chose is kept like any app's setting). A custom profile name is never selected by the core: it
//!   selects `ask` and says so (`accessModeLowered`).

use std::sync::Arc;

use serde_json::{Value, json};
use tracing::warn;

use crate::access::FALLBACK_PROFILE;
use crate::command::Origin;
use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::{AlertKind, CoreEvent};
use crate::sessions;

/// A session being restored after a resume; ends the restore when dropped (also on an error).
pub(crate) struct Restoring {
    shared: Arc<Shared>,
    session_id: String,
    /// The profile to put back, resolved before the resume is sent.
    wanted: String,
    model: Option<crate::model_choices::Choice>,
    /// `wanted` came from this app's own record (`access.rs`: memory or `access.json` in the app's data
    /// folder), not from Vibe's session file or a caller. Only then is Full access put back without a dialog.
    ours: bool,
    /// Held through verification; another open or runtime read cannot cross the restore.
    _held: tokio::sync::OwnedMutexGuard<()>,
    pub(crate) attached: bool,
}

impl Drop for Restoring {
    fn drop(&mut self) {
        self.shared.channels.end_restore(&self.session_id);
    }
}

impl Restoring {
    /// The selected profile, or Ask when the gate requires a new confirmation for it.
    pub(crate) fn profile(&self) -> &str {
        let params = json!({"sessionId": self.session_id, "agentName": self.wanted});
        if self
            .shared
            .gate
            .check("session/agent/update", &params, None)
            .is_ok()
        {
            &self.wanted
        } else {
            FALLBACK_PROFILE
        }
    }
}

/// A session's resolved display mode, from the app's record first. This does not grant a usable
/// channel: the same choice still has to pass the verified restore before commands can run.
pub(crate) fn display_mode(shared: &Shared, id: &str, session: &Value) -> Option<String> {
    let remembered = shared.access.remembered(id);
    let ours = remembered.is_some();
    let wanted = remembered.or_else(|| {
        session
            .pointer("/agent/name")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })?;
    let allowed = (ours && wanted == "auto-approve")
        || shared
            .gate
            .check(
                "session/agent/update",
                &json!({"sessionId": id, "agentName": wanted}),
                None,
            )
            .is_ok();
    Some(if allowed {
        wanted
    } else {
        FALLBACK_PROFILE.to_owned()
    })
}

impl Core {
    /// Runtime reads of a composer opened during restoration follow the verified switch.
    pub(crate) async fn runtime_restore_lock(
        &self,
        method: &str,
        params: &Value,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        if method == "runtime/read"
            && let Some(id) = params["sessionId"].as_str()
        {
            Some(self.shared.access.lock_restore(id).await)
        } else {
            None
        }
    }

    /// Marks `session_id` as being restored and resolves the profile to put back (see the module doc;
    /// `asked` is the profile the caller's resume names, which wins). Call before the resume.
    pub(crate) async fn begin_restore(&self, session_id: &str, asked: Option<String>) -> Restoring {
        let held = self.shared.access.lock_restore(session_id).await;
        let attached = self.shared.channels.has_channel(session_id);
        self.shared.channels.begin_restore(session_id);
        let remembered = self.shared.access.remembered(session_id);
        let recorded = self.recorded_session(session_id).await;
        let model = recorded.as_ref().and_then(crate::model_choices::recorded);
        let (wanted, ours) = match (asked, remembered) {
            (Some(agent), _) => (agent, false),
            (None, Some(agent)) => (agent, true),
            (None, None) => (
                recorded
                    .as_ref()
                    .and_then(|session| {
                        session
                            .pointer("/agent/name")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| FALLBACK_PROFILE.to_owned()),
                false,
            ),
        };
        Restoring {
            shared: self.shared.clone(),
            session_id: session_id.to_owned(),
            wanted,
            model,
            ours,
            _held: held,
            attached,
        }
    }

    /// Read stored pins before resume skips unavailable model overrides.
    /// This read grants no access; the profile still requires the verified switch below.
    async fn recorded_session(&self, session_id: &str) -> Option<Value> {
        let page = json!({"limit": 1});
        let params = json!({"sessionId": session_id, "history": page, "turns": page});
        match self.shared.gate.call("session/read", params, None).await {
            Ok(reply) => reply.result.pointer("/state/session").cloned(),
            Err(error) => {
                warn!(session_id, %error, "reading the session's recorded profile/model failed");
                self.shared
                    .store
                    .view(session_id)
                    .map(|view| view.state["session"].clone())
            }
        }
    }

    /// Switches a resumed session to the profile `restore` resolved (or `ask` when the core may not
    /// select that one itself) and checks that Vibe reports it active. Sent straight through the gate
    /// (not `forward`: the session is "being restored" and `forward` would open it again). On any
    /// failure the session's channel is closed and the command refused (fail closed).
    pub(crate) async fn restore_profile(
        &self,
        origin: Origin,
        restore: Restoring,
    ) -> Result<(), CoreError> {
        let mut restore = restore;
        let actual = self.shared.channels.canonical(&restore.session_id);
        if actual != restore.session_id {
            crate::handoff::handed_off(&self.shared, &restore.session_id, &actual).await;
            self.shared.channels.end_restore(&restore.session_id);
            self.shared.channels.begin_restore(&actual);
            restore.session_id = actual;
        }
        let id = restore.session_id.clone();
        let wanted = restore.wanted.clone();
        let method = "session/agent/update";
        let switch = |agent: &str| json!({"sessionId": id, "agentName": agent});
        let mut token = None;
        let mut lowered = self
            .shared
            .gate
            .check(method, &switch(&wanted), None)
            .is_err();
        if lowered && restore.ours && wanted == "auto-approve" {
            // The user's own earlier choice for this session: re-select it with a token bound to this
            // exact switch. A refusal (the gate does not know the call) keeps the fallback below.
            match self
                .shared
                .gate
                .request_trusted_confirmation(method, &switch(&wanted))
            {
                Ok(minted) => {
                    token = Some(minted);
                    lowered = false;
                }
                Err(error) => warn!(session_id = id, %error, "auto-approve could not be put back"),
            }
        }
        let agent = if lowered {
            FALLBACK_PROFILE
        } else {
            wanted.as_str()
        };
        let params = switch(agent);
        let reply = match self
            .shared
            .gate
            .call(method, params.clone(), token.as_ref())
            .await
        {
            Ok(reply) => reply,
            Err(error) => {
                warn!(session_id = id, %error, "switching a resumed session's profile failed");
                return Err(self.fail_closed(restore, agent).await);
            }
        };
        let active = reply
            .result
            .pointer("/runtime/activeAgent/name")
            .and_then(Value::as_str);
        if !reports_agent(&reply.result, agent) {
            warn!(
                session_id = id,
                asked = agent,
                active,
                "Vibe reports another profile after the switch"
            );
            return Err(self.fail_closed(restore, agent).await);
        }
        sessions::observe(&self.shared, method, &params, &reply.result).await?;
        let runtime = crate::model_choices::restore(
            &self.shared,
            &id,
            reply.result["runtime"].clone(),
            restore.model.clone(),
        )
        .await;
        // Publish the verified runtime even when Vibe only returns it in the switch's reply.
        let applied = self.shared.store.apply(vw_session::Notification::new(
            "runtime/updated",
            json!({"sessionId": id, "runtime": runtime}),
        ));
        let _ = self.shared.loads.send(applied).await;
        // No full-access alert here: reopening puts back the mode the session had; the alert belongs to
        // the user's own switch up (`full_access_alert.rs`).
        drop(restore);
        if lowered {
            warn!(
                origin = origin.as_str(),
                session_id = id,
                from = wanted,
                "a resumed session was put in ask instead of its profile"
            );
            self.shared.emit(CoreEvent::Alert {
                session_id: Some(id.clone()),
                kind: AlertKind::AccessModeLowered,
                message: format!(
                    "“{}” was reopened in Ask. Choose the access again in the chat if you want to change it.{}",
                    self.shared.names.title_of(&self.shared.store, &id).unwrap_or_else(|| "Untitled session".to_owned()),
                    if wanted == "plan" { format!("\nPlan first is gone in Vibe 3; {} is Vibe Work’s planning mode.", vw_vocabulary::label("accessModes", "vw-plan")) } else { String::new() }
                ),
            });
        }
        Ok(())
    }

    /// Closes the session's channel while it is still "being restored" (so nothing can use it), tells
    /// the user, and returns the error for the command.
    async fn fail_closed(&self, restore: Restoring, agent: &str) -> CoreError {
        let id = restore.session_id.clone();
        self.shared.channels.close(&id).await;
        drop(restore);
        warn!(
            session_id = id,
            mode = agent,
            "access restore failed; channel closed and nothing sent"
        );
        let title = self
            .shared
            .names
            .title_of(&self.shared.store, &id)
            .unwrap_or_else(|| "Untitled session".to_owned());
        let mode = vw_vocabulary::term("accessModes", agent)
            .map_or("the previous access mode", |mode| mode.name);
        let message = format!(
            "Couldn't set “{title}” back to {mode}. Nothing was sent. Choose the access again in the chat."
        );
        self.shared.emit(CoreEvent::Alert {
            session_id: Some(id),
            kind: AlertKind::AccessModeUnverified,
            message: message.clone(),
        });
        CoreError::new(ErrorCode::AccessModeUnverified, message).with_method("session/agent/update")
    }
}

fn reports_agent(result: &Value, wanted: &str) -> bool {
    result
        .pointer("/runtime/activeAgent/name")
        .and_then(Value::as_str)
        == Some(wanted)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn profile_verification_rejects_wrong_missing_and_malformed_names() {
        for value in [
            json!({"runtime":{"activeAgent":{"name":"auto-approve"}}}),
            json!({}),
            json!({"runtime":{"activeAgent":{"name":42}}}),
        ] {
            assert!(!super::reports_agent(&value, "ask"), "{value}");
        }
        assert!(super::reports_agent(
            &json!({"runtime":{"activeAgent":{"name":"ask"}}}),
            "ask"
        ));
    }
    struct Decline;
    impl vw_gate::Confirmer for Decline {
        fn confirm(
            &self,
            _: &vw_gate::ConfirmationRequest,
        ) -> impl std::future::Future<Output = vw_gate::Confirmation> + Send {
            std::future::ready(vw_gate::Confirmation::Declined)
        }
    }
    #[tokio::test]
    async fn foreign_metadata_is_shown_with_full_and_unknown_access_lowered_without_vibe() {
        let dir = std::env::temp_dir().join(format!("vw-metadata-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = crate::CoreConfig {
            start_vibe: false,
            ..crate::CoreConfig::default()
        };
        let core = crate::Core::start(
            vw_supervisor::SupervisorConfig::new(
                dir.join("no-server-requested"),
                dir.join("vibe-home"),
                dir.clone(),
            ),
            std::sync::Arc::new(vw_supervisor::NoSecret),
            Decline,
            config,
        )
        .await
        .unwrap();
        for (recorded, wanted) in [("plan", "ask"), ("auto-approve", "ask"), ("custom", "ask")] {
            assert_eq!(
                super::display_mode(
                    &core.shared,
                    "external",
                    &json!({"agent":{"name":recorded}})
                )
                .as_deref(),
                Some(wanted)
            );
        }
        assert!(core.status().server.pid.is_none());
        core.shutdown().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
