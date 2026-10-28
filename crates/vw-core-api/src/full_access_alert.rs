//! The full-access alert (`AlertKind::BypassToolPermissions`): raised once when the user raises a session,
//! or the default profile for new sessions, from a lower mode to Full access.
//!
//! Full access is `auto-approve` by name, or a profile Vibe reports as running every tool without asking
//! (`bypassToolPermissions`: a custom profile TOML can redefine a name such as `ask`). Reopening a session
//! puts its mode back and raises nothing (`restore.rs` never calls here); a session that starts under a
//! Full access default raises nothing either (a start is not a switch).
//!
//! Depends on: `core` (`Core`, `Shared`), `access` (remembered profiles, the default record), `event`.
//! Called by: `dispatch::forward_with_access_switch` (before the gate call: [`Core::full_before`]; after an
//! accepted call: [`Core::after_access_write`]).

use serde_json::{Value, json};
use tracing::warn;

use crate::command::Origin;
use crate::core::{Core, Shared};
use crate::event::{AlertKind, CoreEvent};

impl Core {
    /// For a session's switch (`session/agent/update`) or a write of the default profile for new sessions:
    /// whether it was Full access before (the session's mode, or the default as this core last wrote it).
    /// `None` for every other call. Sessions that start under a Full access default raise no alert: a
    /// start is neither.
    pub(crate) fn full_before(&self, method: &str, params: &Value) -> Option<bool> {
        if method == "session/agent/update" {
            Some(in_full_access(&self.shared, params))
        } else {
            crate::access::default_written(method, params)
                .map(|_| self.shared.access.default_is_full())
        }
    }

    /// The full-access alert after an accepted switch or default write that [`Self::full_before`] covered.
    pub(crate) async fn after_access_write(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
        unconfirmed: bool,
        was_full: bool,
    ) {
        if let Some(profile) = crate::access::default_written(method, params) {
            self.default_written(origin, params, &profile, unconfirmed, was_full)
                .await;
        } else if !was_full {
            self.alert_full_access(origin, method, params, unconfirmed)
                .await;
        }
    }

    /// After an accepted write of the default profile for new sessions: records whether the new default
    /// runs every tool without asking and raises the full-access alert once when it was not so before.
    /// `auto-approve` is Full access by name. For another profile, written without a confirmation, the
    /// only answer Vibe gives is the runtime of a session that runs it: the writing session's runtime is
    /// read when that session runs the profile just made the default; otherwise it counts as not Full.
    async fn default_written(
        &self,
        origin: Origin,
        params: &Value,
        profile: &str,
        unconfirmed: bool,
        was_full: bool,
    ) {
        let session_id = params.get("sessionId").and_then(Value::as_str);
        let full = if profile == "auto-approve" {
            true
        } else if let Some(id) = session_id.filter(|id| {
            unconfirmed && self.shared.access.remembered(id).as_deref() == Some(profile)
        }) {
            match self
                .shared
                .gate
                .call("runtime/read", json!({"sessionId": id}), None)
                .await
            {
                Ok(reply) => bypasses(&reply.result["runtime"]),
                Err(error) => {
                    warn!(session_id = id, %error, "runtime/read after a default profile write failed");
                    false
                }
            }
        } else {
            false
        };
        self.shared.access.set_default_full(full);
        if full && !was_full {
            warn!(
                origin = origin.as_str(),
                profile, "the default profile bypasses tool permissions"
            );
            self.shared.emit(CoreEvent::Alert {
                session_id: session_id.map(str::to_owned),
                kind: AlertKind::BypassToolPermissions,
                message: "The default access mode for new sessions runs every tool without asking (its profile turns tool permissions off).".to_owned(),
            });
        }
    }

    /// Raises the full-access alert after the user's switch of a session to Full access from a lower
    /// mode (the caller checked the mode before). Selecting `auto-approve` is Full access; another profile
    /// counts when Vibe reports that it runs every tool without asking, which is read only after a
    /// switch that needed no confirmation. A failed read is logged, not raised: the switch itself worked.
    /// Never called for reopening a session (its access mode is put back, not chosen); a default write
    /// goes to `default_written`.
    async fn alert_full_access(
        &self,
        origin: Origin,
        method: &str,
        params: &Value,
        unconfirmed: bool,
    ) {
        let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let full = if vw_gate::full_access_escalation(method, params) {
            true
        } else if unconfirmed {
            let read = self
                .shared
                .gate
                .call("runtime/read", json!({"sessionId": session_id}), None)
                .await;
            match read {
                Ok(reply) => bypasses(&reply.result["runtime"]),
                Err(error) => {
                    warn!(session_id, %error, "runtime/read after a profile switch failed");
                    false
                }
            }
        } else {
            false
        };
        if full {
            warn!(
                origin = origin.as_str(),
                session_id, "profile switch bypasses tool permissions"
            );
            self.shared.emit(CoreEvent::Alert {
                session_id: Some(session_id.to_owned()),
                kind: AlertKind::BypassToolPermissions,
                message: "This access mode runs every tool without asking (its profile turns tool permissions off).".to_owned(),
            });
        }
    }
}

/// True when a runtime snapshot says its profile runs every tool without asking.
fn bypasses(runtime: &Value) -> bool {
    runtime
        .get("bypassToolPermissions")
        .and_then(Value::as_bool)
        == Some(true)
}

/// Whether the session of a switch already runs in Full access: the profile this core last saw for it
/// is `auto-approve`, or Vibe's latest runtime for that profile runs every tool without asking.
fn in_full_access(shared: &Shared, params: &Value) -> bool {
    let Some(id) = params.get("sessionId").and_then(Value::as_str) else {
        return false;
    };
    profile_is_full(
        shared.access.remembered(id).as_deref(),
        shared.store.runtime(id).as_ref(),
    )
}

fn profile_is_full(profile: Option<&str>, runtime: Option<&Value>) -> bool {
    // A switch reply records the profile before its runtime notification reaches the store.
    profile == Some("auto-approve")
        || runtime.is_some_and(|runtime| {
            (profile.is_none()
                || runtime.pointer("/activeAgent/name").and_then(Value::as_str) == profile)
                && bypasses(runtime)
        })
}

#[cfg(test)]
mod tests {
    use super::profile_is_full;
    use serde_json::json;

    #[test]
    fn an_accepted_lower_mode_overrides_the_previous_full_runtime() {
        let previous = json!({
            "activeAgent": {"name": "auto-approve"},
            "bypassToolPermissions": true
        });
        // The switch reply records Ask before its runtime notification reaches the store.
        assert!(!profile_is_full(Some("ask"), Some(&previous)));
        assert!(!profile_is_full(Some("plan"), Some(&previous)));
    }

    #[test]
    fn a_current_bypassing_profile_still_counts_as_full() {
        let current = json!({
            "activeAgent": {"name": "ask"},
            "bypassToolPermissions": true
        });
        assert!(profile_is_full(Some("ask"), Some(&current)));
        assert!(profile_is_full(None, Some(&current)));
        assert!(profile_is_full(Some("auto-approve"), None));
        assert!(!profile_is_full(Some("ask"), None));
        assert!(!profile_is_full(
            Some("ask"),
            Some(&json!({"activeAgent": {"name": "ask"}, "bypassToolPermissions": false}))
        ));
    }
}
