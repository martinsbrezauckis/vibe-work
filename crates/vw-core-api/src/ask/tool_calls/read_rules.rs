//! Whether the person set a stricter rule for a Vibe Work tool. Tools accepted in Accept edits run without
//! the default approval card (`tool_calls::accepts_edits`); a rule the person configured in Vibe (the tool, or the
//! file read it stands for, set to ask or never, or turned off) keeps the card.
//!
//! Invariants:
//! - The same persisted layers Vibe merges for the chat are read: Vibe's own config, the active agent and approval
//!   policy files, each folder's `.vibe` config and profiles up to the Vibe home's parent, cached administrator rules and the
//!   `VIBE_TOOLS` override. Any layer that names a stricter rule wins (conservative union).
//! - A layer that cannot be read or parsed, or a check that runs out of time, counts as a stricter rule: the card
//!   stays. Nothing read here is shown or returned; only the yes/no answer leaves this module.
//!
//! Depends on: `search_read_rules::read` (bounded, no-follow config reads). Called by: `tool_calls::permission`.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vw_agent_tools::wire::{SERVER_NAME, tools as names};

/// The Vibe tool settings that govern `tool`: its own (Vibe publishes it as `vw_<tool>`), and for a tool that reads
/// the chat's files the built-in file reads it stands for.
fn governing(tool: &str) -> Vec<String> {
    let mut list = vec![format!("{SERVER_NAME}_{tool}")];
    match tool {
        names::SEARCH => list.extend(["read_file".to_owned(), "grep".to_owned()]),
        names::PANEL_READ | names::HANDBOOK_CHECK | names::HANDBOOK_ANSWERS => {
            list.push("read_file".to_owned());
        }
        _ => {}
    }
    list
}

/// True when a layer sets one of `tools` to ask or never, or turns it off.
fn stricter_in(text: &str, tools: &[String]) -> Result<bool, ()> {
    let table: toml::Table = text.parse().map_err(|_| ())?;
    Ok(stricter_table(&table, tools))
}

fn stricter_table(table: &toml::Table, tools: &[String]) -> bool {
    let settings = table.get("tools").and_then(toml::Value::as_table);
    let disabled = table.get("disabled_tools").and_then(toml::Value::as_array);
    tools.iter().any(|tool| {
        settings
            .and_then(|s| s.get(tool))
            .and_then(|t| t.get("permission"))
            .and_then(toml::Value::as_str)
            .is_some_and(|p| matches!(p, "ask" | "never"))
            || disabled.is_some_and(|d| d.iter().any(|v| v.as_str() == Some(tool.as_str())))
    })
}

/// The files of one folder level: its config and the selected agent and approval policy.
fn layer_files(dir: &Path, mode: &str, agent: Option<&str>) -> Vec<PathBuf> {
    let mut files = vec![
        dir.join("config.toml"),
        dir.join("agents").join(format!("{mode}.toml")),
    ];
    if let Some(agent) = agent.filter(|agent| *agent != mode) {
        files.push(dir.join("agents").join(format!("{agent}.toml")));
    }
    files
}

/// Whether the person configured a stricter rule than the default for `tool` in `mode` (module docs). Blocking.
pub(super) fn stricter(
    home: &Path,
    roots: &[PathBuf],
    mode: &str,
    agent: Option<&str>,
    tool: &str,
) -> bool {
    check(
        home,
        roots,
        mode,
        agent,
        tool,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_or(true)
}

fn check(
    home: &Path,
    roots: &[PathBuf],
    mode: &str,
    agent: Option<&str>,
    tool: &str,
    deadline: Instant,
) -> Result<bool, ()> {
    if !valid_profile(mode) || agent.is_some_and(|agent| !valid_profile(agent)) {
        return Err(());
    }
    let tools = governing(tool);
    let mut files = layer_files(home, mode, agent);
    for root in roots {
        for ancestor in root.ancestors().take(64) {
            files.extend(layer_files(&ancestor.join(".vibe"), mode, agent));
            if Some(ancestor) == home.parent() {
                break;
            }
        }
    }
    for file in files {
        if Instant::now() >= deadline {
            return Err(());
        }
        if let Some(text) = crate::search_read_rules::read(&file).map_err(|_| ())?
            && stricter_in(&text, &tools)?
        {
            return Ok(true);
        }
    }
    if let Some(text) =
        crate::search_read_rules::read(&home.join("managed_config_cache.json")).map_err(|_| ())?
    {
        let cache: serde_json::Value = serde_json::from_str(&text).map_err(|_| ())?;
        for entry in cache.as_object().into_iter().flat_map(|m| m.values()) {
            if let Some(payload) = entry["payload"].as_str()
                && stricter_in(payload, &tools)?
            {
                return Ok(true);
            }
        }
    }
    // Vibe's environment override holds the `tools` table itself (as JSON).
    if let Ok(text) = std::env::var("VIBE_TOOLS") {
        let settings: toml::Table = serde_json::from_str(&text).map_err(|_| ())?;
        let mut table = toml::Table::new();
        table.insert("tools".into(), toml::Value::Table(settings));
        if stricter_table(&table, &tools) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn valid_profile(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !matches!(name, "." | "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature_test_core::Fixture;
    use serde_json::json;

    #[tokio::test]
    async fn active_agent_rule_keeps_the_accept_edits_tool_card() {
        let f = Fixture::new().await;
        let project = f.root.join("project");
        f.load("chat", &project, &[], false);
        let agents = project.join(".vibe/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("explore.toml"),
            "[tools.read_file]\npermission = 'ask'",
        )
        .unwrap();
        let default = json!({"activeAgent":{"name":"default"},
            "activeApprovalPolicy":{"name":"accept-edits"}});
        let _ = f.core.shared.store.apply(vw_session::Notification::new(
            "runtime/updated",
            json!({"sessionId":"chat","runtime":default}),
        ));
        f.core.shared.access.observe_runtime("chat", &default);
        assert!(!super::super::stricter_rule(&f.core.shared, "chat", names::PANEL_READ).await);
        let explore = json!({"activeAgent":{"name":"explore"},
            "activeApprovalPolicy":{"name":"accept-edits"}});
        let _ = f.core.shared.store.apply(vw_session::Notification::new(
            "runtime/updated",
            json!({"sessionId":"chat","runtime":explore}),
        ));
        f.core.shared.access.observe_runtime("chat", &explore);
        assert!(super::super::stricter_rule(&f.core.shared, "chat", names::PANEL_READ).await);
        f.finish().await;
    }

    #[tokio::test]
    async fn active_ask_policy_rule_keeps_the_browser_read_card() {
        let f = Fixture::new().await;
        let project = f.root.join("project");
        f.load("chat", &project, &[], false);
        let agents = project.join(".vibe/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ask.toml"),
            "[tools.vw_browser_read]\npermission = 'ask'",
        )
        .unwrap();
        let runtime = json!({"activeAgent":{"name":"default"},
            "activeApprovalPolicy":{"name":"ask"}});
        let _ = f.core.shared.store.apply(vw_session::Notification::new(
            "runtime/updated",
            json!({"sessionId":"chat","runtime":runtime}),
        ));
        f.core.shared.access.observe_runtime("chat", &runtime);
        assert!(super::super::stricter_rule(&f.core.shared, "chat", names::BROWSER_READ).await);
        f.finish().await;
    }

    #[test]
    fn ask_never_and_off_are_stricter_and_unreadable_config_keeps_the_card() {
        let search = governing(names::SEARCH);
        for (text, stricter) in [
            ("", false),
            ("[tools.read_file]\npermission = \"always\"", false),
            ("[tools.read_file]\npermission = \"ask\"", true),
            ("[tools.grep]\npermission = \"never\"", true),
            ("[tools.vw_search]\npermission = \"ask\"", true),
            ("disabled_tools = [\"read_file\"]", true),
            ("[tools.vw_preview_list]\npermission = \"ask\"", false),
        ] {
            assert_eq!(stricter_in(text, &search), Ok(stricter), "{text}");
        }
        assert!(stricter_in("broken = [", &search).is_err());
        let root = std::env::temp_dir().join(format!("vw-read-card-rules-{}", std::process::id()));
        let home = root.join("vibe-home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "broken = [").unwrap();
        assert!(stricter(
            &home,
            &[],
            "accept-edits",
            None,
            names::PREVIEW_LIST
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
