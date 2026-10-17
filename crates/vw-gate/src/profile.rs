//! Vibe's agent profiles ("access modes") and which of them need a native confirmation.
//!
//! Depends on: `serde_json`. Called by: privileged (`session/agent/update`, `config/write`),
//! `agent_config` (`agentConfig.agent`), core confirmer (Debug API exception).
//! Only the listed profiles are accepted.
//! `lean` "requires installation" and is not accepted. `smart-approve` is a
//! Mistral dark launch: Vibe lists it in `runtime.agents` only for users in the experiment, the access
//! menu offers it only then, and Vibe itself refuses the switch for everyone else ("not
//! available"). It asks for risky calls, so it needs no confirmation (like `accept-edits`).
//! Invariant: selecting the profile that approves everything is an escalation and needs a token.

use serde_json::Value;

/// The profiles the app may select.
pub(crate) const PROFILES: [&str; 7] = [
    "ask",
    "plan",
    "accept-edits",
    "auto-approve",
    "explore",
    "smart-approve",
    // The app-owned profile Vibe Work writes to `VIBE_HOME/agents/vw-plan.toml`
    // (plan rules + the plan pages folders). More limited than `ask` for file writes: no confirmation.
    "vw-plan",
];

/// True when `name` is one of the profiles.
pub(crate) fn is_profile(name: &str) -> bool {
    PROFILES.contains(&name)
}

/// True for the profile that runs every tool without asking (full access).
pub(crate) fn escalates(name: &str) -> bool {
    name == "auto-approve"
}

/// Whether a confirmation request enables Full access. The gate validates the complete call before
/// asking; this predicate names the escalation for the dialog's wording (the window and the agent are asked; a Debug API
/// request is answered by the core's confirmer like every other non-vault one).
#[must_use]
pub fn full_access_escalation(method: &str, params: &Value) -> bool {
    if method == "session/agent/update" {
        return params.get("agentName").and_then(Value::as_str) == Some("auto-approve");
    }
    if method == "config/write" {
        return params
            .get("ops")
            .and_then(Value::as_array)
            .is_some_and(|ops| {
                ops.iter().any(|op| {
                    op.get("op").and_then(Value::as_str) == Some("set")
                        && op.get("path").and_then(Value::as_str) == Some("/default_agent")
                        && op.get("value").and_then(Value::as_str) == Some("auto-approve")
                })
            });
    }
    if !matches!(
        method,
        "session/start" | "session/resume" | "session/fork" | "session/continue"
    ) {
        return false;
    }
    params.get("agentConfig").is_some_and(|config| {
        config.get("agent").and_then(Value::as_str) == Some("auto-approve")
            || config.get("autoApprove").and_then(Value::as_bool) == Some(true)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::full_access_escalation;

    #[test]
    fn finds_every_full_access_entry_without_classifying_other_profiles() {
        assert!(full_access_escalation(
            "session/agent/update",
            &json!({"sessionId": "s", "agentName": "auto-approve"})
        ));
        for method in [
            "session/start",
            "session/resume",
            "session/fork",
            "session/continue",
        ] {
            assert!(full_access_escalation(
                method,
                &json!({"agentConfig": {"agent": "auto-approve"}})
            ));
            assert!(full_access_escalation(
                method,
                &json!({"agentConfig": {"autoApprove": true}})
            ));
            assert!(!full_access_escalation(
                method,
                &json!({"agentConfig": {"agent": "plan", "autoApprove": false}})
            ));
        }
        assert!(full_access_escalation(
            "config/write",
            &json!({"ops": [{"op": "set", "path": "/default_agent", "value": "auto-approve"}]})
        ));
        assert!(!full_access_escalation(
            "config/write",
            &json!({"ops": [{"op": "remove", "path": "/default_agent"}]})
        ));
        assert!(!full_access_escalation(
            "unrelated/method",
            &json!({"agentConfig": {"autoApprove": true}})
        ));
    }
}
