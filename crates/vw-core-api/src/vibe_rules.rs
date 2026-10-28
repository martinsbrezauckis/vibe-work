//! Old "always" answers in Vibe's config, visible and removable. Before 0.1.2 Vibe's "Always allow" on an approval card made Vibe write a lasting rule into
//! Vibe Work's own `config.toml` (`VIBE_HOME`, Vibe's user layer, `_unified_permissions.py` `_persist`):
//! `/tools/<tool>/permission = "always"` for a tool without permissions, or the call's patterns appended to
//! `/tools/<tool>/allowlist`. They keep working; this module lists them and removes one.
//!
//! Reading: Vibe's `config/fields/read` hides `tools` (`HIDDEN_SETTINGS`, `_config_introspect.py:40-48`), so the
//! list reads the file Vibe Work already reads and writes for the default model (`default_model.rs`). Vibe writes an
//! allowlist as its whole effective list (its defaults included, `vibe_schema.py:901-926`), so the entries that an
//! "Always allow" added are those not in the tool's default allowlist; the defaults come from pinned Vibe's own
//! tool catalogue in its isolated Python (`vibe_tool_defaults.py`). Without that Python every entry is listed and
//! `defaultsKnown` is false.
//!
//! Removing: through Vibe's config API (`config/write` in a short-lived helper chat, like `subagents_setting.rs`),
//! with the gate's typed `remove_tool_rule` (the client table refuses every `/tools` write). Only the window and
//! the Debug API (acting as the user) reach this command; several rules go in one write (`removeMany`); the agent has no route. A rule Vibe already remembers in
//! a running session's memory stays until that session ends; new sessions ask again.
//!
//! Depends on: `toml_edit`, `plugin_identity` (Vibe's Python runner), `helper_chat`, the gate. Called by:
//! `dispatch` (`vibeRules`).
use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml_edit::DocumentMut;
use vw_gate::ToolRuleRemoval;

use crate::command::{CommandResult, Origin};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// One `vibeRules` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VibeRulesRequest {
    /// The lasting rules earlier "Always allow" answers left in Vibe's config.
    List {},
    /// Removes one: the tool's `always` permission (`pattern` absent) or one allowlist pattern.
    Remove {
        /// Vibe's tool name.
        tool: String,
        /// The allowlist pattern; absent for the tool-wide permission.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        pattern: Option<String>,
    },
    /// Removes several in one config write ("Revoke all"). Rules already gone are skipped.
    RemoveMany {
        /// The rules, as `list` returned them (at most 1024).
        rules: Vec<RuleRef>,
    },
}

/// A rule named for removal: the tool, and the pattern unless it is the tool-wide permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleRef {
    /// Vibe's tool name.
    pub tool: String,
    /// The allowlist pattern; absent (or null) for the tool-wide permission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pattern: Option<String>,
}

/// One lasting rule in Vibe's config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VibeRule {
    /// Vibe's tool name.
    pub tool: String,
    /// The allowlist pattern; absent: every call of the tool is allowed (`permission = "always"`).
    pub pattern: Option<String>,
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

/// The user layer's `[tools.<name>]` permission and allowlist, from Vibe Work's own `config.toml`.
fn read_tools(path: &std::path::Path) -> Result<BTreeMap<String, (bool, Vec<String>)>, CoreError> {
    let text = match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_CONFIG_BYTES => {
            return Err(CoreError::new(
                ErrorCode::Internal,
                "Vibe's config file is too large to read.",
            ));
        }
        Ok(_) => std::fs::read_to_string(path)
            .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))?,
        Err(_) => return Ok(BTreeMap::new()),
    };
    let doc: DocumentMut = text.parse().map_err(|_| {
        CoreError::new(ErrorCode::Internal, "Vibe's config file is not valid TOML.")
    })?;
    let mut tools = BTreeMap::new();
    let Some(table) = doc.get("tools").and_then(|t| t.as_table_like()) else {
        return Ok(tools);
    };
    for (name, item) in table.iter() {
        let Some(tool) = item.as_table_like() else {
            continue;
        };
        let always = tool.get("permission").and_then(|p| p.as_str()) == Some("always");
        let allowlist = tool
            .get("allowlist")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        tools.insert(name.to_owned(), (always, allowlist));
    }
    Ok(tools)
}

impl Core {
    fn vibe_config(&self) -> std::path::PathBuf {
        self.shared.instructions.own_home.join("config.toml")
    }

    /// Each tool's default allowlist from pinned Vibe, or `None` when its Python cannot be run.
    fn vibe_tool_defaults(&self) -> Option<BTreeMap<String, Vec<String>>> {
        let python = self.shared.plugin_approvals.vibe_python()?;
        let home = &self.shared.instructions.own_home;
        let output = crate::plugin_identity::run_vibe_python(
            &python,
            include_str!("vibe_tool_defaults.py"),
            &[],
            home,
            Duration::from_secs(30),
            MAX_CONFIG_BYTES,
        )
        .inspect_err(|reason| tracing::warn!(%reason, "Vibe's tool defaults could not be read"))
        .ok()?;
        serde_json::from_slice(&output).ok()
    }

    /// The lasting rules, and whether Vibe's defaults were known (so only added patterns are listed).
    fn vibe_rules_now(&self) -> Result<(Vec<VibeRule>, bool), CoreError> {
        let tools = read_tools(&self.vibe_config())?;
        let defaults = if tools.values().any(|(_, list)| !list.is_empty()) {
            self.vibe_tool_defaults()
        } else {
            Some(BTreeMap::new())
        };
        let mut rules = Vec::new();
        for (tool, (always, allowlist)) in &tools {
            if *always {
                rules.push(VibeRule {
                    tool: tool.clone(),
                    pattern: None,
                });
            }
            let shipped = defaults.as_ref().and_then(|d| d.get(tool));
            for pattern in allowlist {
                if !shipped.is_some_and(|d| d.contains(pattern)) {
                    rules.push(VibeRule {
                        tool: tool.clone(),
                        pattern: Some(pattern.clone()),
                    });
                }
            }
        }
        Ok((rules, defaults.is_some()))
    }

    /// `vibeRules`: list the lasting rules, or remove one or several through Vibe's config API (one write).
    ///
    /// # Errors
    /// `invalidParams` when the rule is not (or no longer) in Vibe's config; Vibe's refusal of the write; the helper
    /// chat's errors.
    pub(crate) async fn vibe_rules(
        &self,
        origin: Origin,
        request: VibeRulesRequest,
    ) -> Result<CommandResult, CoreError> {
        let wanted = match request {
            VibeRulesRequest::List {} => Vec::new(),
            VibeRulesRequest::Remove { tool, pattern } => vec![RuleRef { tool, pattern }],
            VibeRulesRequest::RemoveMany { rules } => {
                if rules.is_empty() || rules.len() > 1024 {
                    return Err(invalid("Name between 1 and 1024 rules to remove."));
                }
                rules
            }
        };
        let mut removed = 0;
        if !wanted.is_empty() {
            // Held from the read to the write: the write is the list read minus entries, so a second removal that
            // read the same list would otherwise put an entry back (b1r).
            let _one_at_a_time = self.shared.vibe_rules_lock.lock().await;
            let tools = read_tools(&self.vibe_config())?;
            let (removals, count) = removals_for(&tools, &wanted);
            if removals.is_empty() {
                return Err(invalid(if wanted.len() == 1 {
                    "This rule is no longer in Vibe's settings."
                } else {
                    "These rules are no longer in Vibe's settings."
                }));
            }
            self.write_removals(origin, &removals).await?;
            tracing::info!(?removals, "lasting Vibe tool rules removed");
            removed = count;
        }
        // Reading Vibe's defaults runs its Python (up to 30 s): off the async workers.
        let core = self.clone();
        let (rules, defaults_known) = tokio::task::spawn_blocking(move || core.vibe_rules_now())
            .await
            .map_err(|e| CoreError::new(ErrorCode::Internal, e.to_string()))??;
        Ok(CommandResult::VibeRules {
            rules,
            defaults_known,
            removed: u32::try_from(removed).unwrap_or(u32::MAX),
        })
    }

    /// Sends `removals` as one `config/write` through one short-lived helper chat.
    async fn write_removals(
        &self,
        origin: Origin,
        removals: &[ToolRuleRemoval],
    ) -> Result<(), CoreError> {
        let id = self
            .start_helper_chat(origin, "The rule was not removed")
            .await?;
        let written = self.shared.gate.remove_tool_rules(&id, removals).await;
        let _ = self.end_helper_chat(&id).await;
        let reply = written.map_err(|e| CoreError::from_gate("config/write", &e))?;
        if reply.result.get("rejected").and_then(Value::as_bool) == Some(true)
            || reply
                .result
                .get("failures")
                .and_then(Value::as_array)
                .is_some_and(|f| !f.is_empty())
        {
            return Err(
                CoreError::new(ErrorCode::ServerError, "Vibe did not remove the rule.")
                    .with_method("config/write"),
            );
        }
        Ok(())
    }
}

/// The config writes that remove the `wanted` rules still in `tools` (rules already gone are skipped), and how many
/// rules they remove: per tool at most one permission removal and one allowlist write (the current list minus every
/// named pattern, the rest in order).
fn removals_for(
    tools: &BTreeMap<String, (bool, Vec<String>)>,
    wanted: &[RuleRef],
) -> (Vec<ToolRuleRemoval>, usize) {
    let mut removals = Vec::new();
    let mut count = 0;
    for (tool, (always, allowlist)) in tools {
        let named = wanted.iter().filter(|r| r.tool == *tool);
        let permission = *always && named.clone().any(|r| r.pattern.is_none());
        let patterns: Vec<&String> = named
            .filter_map(|r| r.pattern.as_ref())
            .filter(|p| allowlist.contains(p))
            .collect();
        if permission {
            removals.push(ToolRuleRemoval::Permission { tool: tool.clone() });
            count += 1;
        }
        if !patterns.is_empty() {
            let keep: Vec<String> = allowlist
                .iter()
                .filter(|p| !patterns.contains(p))
                .cloned()
                .collect();
            count += allowlist.len() - keep.len();
            removals.push(ToolRuleRemoval::Allowlist {
                tool: tool.clone(),
                keep,
            });
        }
    }
    (removals, count)
}
