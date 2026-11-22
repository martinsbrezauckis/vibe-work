//! Vibe's separate agents and approval policies, and which choices need native confirmation.
//!
//! Depends on: `serde_json`. Called by: privileged (`session/agent/update`, `config/write`),
//! `agent_config` (`agentConfig`), core confirmer (Debug API exception).
//! Only the listed agents and policies are accepted.
//! `lean` "requires installation" and is not accepted. `smart-approve` is a
//! Mistral dark launch: Vibe lists it in `runtime.approvalPolicies` only for users in the experiment, the access
//! menu offers it only then, and Vibe itself refuses the switch for everyone else ("not
//! available"). It asks for risky calls, so it needs no confirmation (like `accept-edits`).
//! Invariant: selecting the policy that approves everything is an escalation and needs a token.

use serde_json::Value;

/// The approval policies the app may select.
pub(crate) const POLICIES: [&str; 4] = ["ask", "accept-edits", "auto-approve", "smart-approve"];

/// Agents Vibe Work can select. The optional `lean` agent needs an installation and is not offered.
pub(crate) const AGENTS: [&str; 2] = ["default", "explore"];

/// True when `name` is an approval policy the app may select.
pub(crate) fn is_policy(name: &str) -> bool {
    POLICIES.contains(&name)
}

/// True when `name` is an agent the app may select.
pub(crate) fn is_agent(name: &str) -> bool {
    AGENTS.contains(&name)
}

/// True for the policy that runs every tool without asking (full access).
pub(crate) fn escalates(name: &str) -> bool {
    name == "auto-approve"
}

/// Whether a confirmation request enables Full access. The gate validates the complete call before
/// asking; this predicate names the escalation for the dialog's wording (the window and the agent are asked; a Debug API
/// request is answered by the core's confirmer like every other non-vault one).
#[must_use]
pub fn full_access_escalation(method: &str, params: &Value) -> bool {
    if method == "session/approvalPolicy/update" {
        return params.get("approvalPolicy").and_then(Value::as_str) == Some("auto-approve");
    }
    // A legacy policy alias on the agent method is still an escalation when an older caller sends
    // it. The new gate refuses the alias, but the core's exception must never mistake it for safe.
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
                        && matches!(
                            op.get("path").and_then(Value::as_str),
                            Some("/default_approval_policy" | "/default_agent")
                        )
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
        config.get("approvalPolicy").and_then(Value::as_str) == Some("auto-approve")
            || config.get("agent").and_then(Value::as_str) == Some("auto-approve")
            || config.get("autoApprove").and_then(Value::as_bool) == Some(true)
    })
}
