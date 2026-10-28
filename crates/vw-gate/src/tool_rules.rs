//! Removing lasting Vibe tool rules (one, or a set in one write): an old "Always allow" that Vibe wrote into Vibe Work's
//! own `config.toml` (`/tools/<tool>/permission = always`, or a pattern in `/tools/<tool>/allowlist`).
//!
//! The client table keeps refusing every `/tools/...` path in `config/write` (`config_rules`), so no window,
//! Debug API or page call can write tool rules. This typed entry point is the one exception, for the core only: it
//! builds the `config/write` itself from a [`ToolRuleRemoval`], which can only remove (the permission key) or
//! write back a list the core computed as the current list minus one entry. The core takes the current list from
//! Vibe Work's own config file right before (`vw-core-api` `vibe_rules`), and only the window and the Debug API
//! (acting as the user) can ask it to; the agent has no route.
//!
//! Depends on: gate (transport), error. Called by: `vw-core-api` `vibe_rules`.
use serde_json::{Value, json};

use crate::error::GateError;
use crate::gate::{Gate, Reply};
use crate::transport::Transport;

/// Most patterns written back in one allowlist.
const MAX_PATTERNS: usize = 1024;
/// Longest pattern written back.
const MAX_PATTERN: usize = 4096;

/// One lasting rule to remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRuleRemoval {
    /// Drop `/tools/<tool>/permission` from Vibe Work's config (the tool falls back to its default).
    Permission {
        /// Vibe's tool name.
        tool: String,
    },
    /// Write `/tools/<tool>/allowlist` back without one entry: `keep` is the current list minus that entry.
    Allowlist {
        /// Vibe's tool name.
        tool: String,
        /// The patterns that stay, in their current order.
        keep: Vec<String>,
    },
}

fn invalid(detail: &str) -> GateError {
    GateError::InvalidParams {
        method: "config/write".to_owned(),
        detail: detail.to_owned(),
    }
}

/// Vibe's tool names are identifiers (`bash`, `read_file`, `data-api_create-issue`): no `/` or `~` that would
/// change the JSON pointer.
fn tool_ok(tool: &str) -> bool {
    !tool.is_empty()
        && tool.len() <= 128
        && tool
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

impl ToolRuleRemoval {
    fn op(&self) -> Result<Value, GateError> {
        match self {
            Self::Permission { tool } if tool_ok(tool) => {
                Ok(json!({"op": "remove", "path": format!("/tools/{tool}/permission")}))
            }
            Self::Allowlist { tool, keep } if tool_ok(tool) => {
                if keep.len() > MAX_PATTERNS || keep.iter().any(|p| p.len() > MAX_PATTERN) {
                    return Err(invalid("the allowlist is too long"));
                }
                Ok(json!({"op": "set", "path": format!("/tools/{tool}/allowlist"), "value": keep}))
            }
            _ => Err(invalid("not a Vibe tool name")),
        }
    }
}

impl<T: Transport, C: crate::Confirmer> Gate<T, C> {
    /// Sends the `config/write` that removes `removal` through the session `session_id` (a short-lived helper
    /// chat of the core). Not reachable from a client command.
    ///
    /// # Errors
    /// [`GateError::InvalidParams`] for a malformed tool name or list (nothing sent); [`GateError::Transport`]
    /// when the send fails.
    pub async fn remove_tool_rule(
        &self,
        session_id: &str,
        removal: &ToolRuleRemoval,
    ) -> Result<Reply, GateError> {
        self.remove_tool_rules(session_id, std::slice::from_ref(removal))
            .await
    }

    /// Sends one `config/write` that removes every rule in `removals` (, "Revoke all"): one write for the whole
    /// set, each removal still only a removal or a list the core computed as the current list minus entries.
    ///
    /// # Errors
    /// [`GateError::InvalidParams`] for an empty or too long set, a malformed tool name or list (nothing sent);
    /// [`GateError::Transport`] when the send fails.
    pub async fn remove_tool_rules(
        &self,
        session_id: &str,
        removals: &[ToolRuleRemoval],
    ) -> Result<Reply, GateError> {
        if session_id.is_empty() || session_id.len() > 128 {
            return Err(invalid("needs the helper session"));
        }
        if removals.is_empty() || removals.len() > MAX_PATTERNS {
            return Err(invalid("between 1 and 1024 rules per write"));
        }
        let ops = removals
            .iter()
            .map(ToolRuleRemoval::op)
            .collect::<Result<Vec<_>, _>>()?;
        let params = json!({
            "sessionId": session_id,
            "ops": ops,
            "reason": "remove lasting tool rules (Vibe Work Settings)",
            "reloadRuntime": true,
        });
        self.send_unchecked("config/write", params).await
    }
}
