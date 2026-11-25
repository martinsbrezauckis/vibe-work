//! Fail-closed admission for command MCP entries in a project's Vibe config.
//!
//! Depends on Vibe's project config discovery order (nearest `.vibe/config.toml`), the core's recorded chat folders,
//! and the shared vocabulary. Called before forwarded requests and live MCP refreshes. The project file is never edited.
//! Until Vibe offers a session-scoped mask for project stdio entries, an enabled entry cannot safely be admitted:
//! folder trust is not command consent and a host-side dialog cannot pin a mutable repository file to Vibe's read.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;
use vw_changes::git::{Call, Index};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_DIRS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Risk {
    Command,
    PluginCommand(String, PathBuf),
    Uncheckable,
    WorktreeMove,
}

fn toml_risk(text: &str) -> Option<Risk> {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return Some(Risk::Uncheckable);
    };
    let Ok(rows) = crate::mcp_rows::read_rows(&doc) else {
        return Some(Risk::Uncheckable);
    };
    for row in rows {
        match row.get("transport").and_then(toml::Value::as_str) {
            // Forced discovery in mcp_catalog/refresh can start a disabled stdio row.
            Some("stdio") => return Some(Risk::Command),
            Some("http" | "streamable-http") if !row.contains_key("command") => {}
            _ => return Some(Risk::Uncheckable),
        }
    }
    None
}

fn plugin_document_risk(bytes: &[u8], name: String, path: PathBuf) -> Option<Risk> {
    let Ok(doc) = serde_json::from_slice::<Value>(bytes) else {
        return Some(Risk::Uncheckable);
    };
    let Some(servers) = doc.get("mcpServers").and_then(Value::as_object) else {
        return Some(Risk::Uncheckable);
    };
    for row in servers.values() {
        match row.get("type").and_then(Value::as_str) {
            Some("stdio") => return Some(Risk::PluginCommand(name, path)),
            Some("http" | "streamable-http" | "sse") => {}
            _ => return Some(Risk::Uncheckable),
        }
    }
    None
}

fn config_risk(folder: &Path) -> Option<Risk> {
    // Vibe 3.1.4 tries the nearest config first. A missing file is ordinary; an unreadable one cannot be
    // certified safe. Inspect both the requested path and its resolved target when a parent is a symlink.
    let resolved = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    for root in [folder, resolved.as_path()] {
        for parent in root.ancestors() {
            let file = parent.join(".vibe/config.toml");
            let metadata = match fs::metadata(&file) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return Some(Risk::Uncheckable),
            };
            if !metadata.is_file() {
                continue;
            }
            if metadata.len() > MAX_CONFIG_BYTES {
                return Some(Risk::Uncheckable);
            }
            let Ok(text) = fs::read_to_string(&file) else {
                return Some(Risk::Uncheckable);
            };
            let risk = toml_risk(&text);
            if risk.is_some() {
                return risk;
            }
            break;
        }
    }
    None
}

fn plugin_risk(folder: &Path) -> Option<Risk> {
    // Vibe's project plugin root is the trusted cwd's own .vibe/plugins directory. User and
    // bundled plugin roots live elsewhere and are deliberately outside this project check.
    let resolved = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    for root in [folder, resolved.as_path()] {
        let directory = root.join(".vibe/plugins");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => return Some(Risk::Uncheckable),
        };
        for (index, entry) in entries.enumerate() {
            if index >= MAX_PLUGIN_DIRS {
                return Some(Risk::Uncheckable);
            }
            let Ok(entry) = entry else {
                return Some(Risk::Uncheckable);
            };
            let plugin = entry.path();
            let Ok(metadata) = fs::metadata(&plugin) else {
                return Some(Risk::Uncheckable);
            };
            if !metadata.is_dir() {
                continue;
            }
            let file = plugin.join("mcp.json");
            let metadata = match fs::metadata(&file) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(_) => return Some(Risk::Uncheckable),
            };
            if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
                return Some(Risk::Uncheckable);
            }
            let Ok(bytes) = fs::read(&file) else {
                return Some(Risk::Uncheckable);
            };
            if let Some(risk) = plugin_document_risk(
                &bytes,
                entry.file_name().to_string_lossy().into_owned(),
                file,
            ) {
                return Some(risk);
            }
        }
    }
    None
}

fn project_risk(folder: &Path) -> Option<Risk> {
    config_risk(folder).or_else(|| plugin_risk(folder))
}

fn absolute(folder: &str, default_cwd: &Path) -> PathBuf {
    let path = Path::new(folder);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        default_cwd.join(path)
    }
}

fn add_workspace_roots(
    roots: &mut HashSet<PathBuf>,
    raw: Option<&Value>,
    default_cwd: &Path,
) -> Result<(), Risk> {
    let Some(raw) = raw else {
        return Ok(());
    };
    let Some(list) = raw.as_array() else {
        return Err(Risk::Uncheckable);
    };
    for root in list {
        let Some(root) = root.as_str().filter(|root| !root.is_empty()) else {
            return Err(Risk::Uncheckable);
        };
        roots.insert(absolute(root, default_cwd));
    }
    Ok(())
}

fn worktree_target(worktree: Option<&Value>, default_cwd: &Path) -> Result<Option<PathBuf>, Risk> {
    let Some(worktree) = worktree.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    match (
        worktree.get("kind").and_then(Value::as_str),
        worktree.get("cwd").and_then(Value::as_str),
    ) {
        (Some("existing"), Some(cwd)) if !cwd.is_empty() => Ok(Some(absolute(cwd, default_cwd))),
        // Only the app's worktrees/start supplies create; it checks the immutable checkout tree
        // before forwarding. A caller-supplied worktree is refused earlier by raw_worktree_refusal.
        (Some("create"), _) => Ok(None),
        _ => Err(Risk::WorktreeMove),
    }
}

fn must_read_unknown_session(method: &str, params: &Value) -> bool {
    crate::routing::loads_plugins(method, params)
        || crate::routing::attaches_session(method, params).is_some()
        || matches!(
            method,
            "mcp_catalog/refresh" | "mcp/refresh" | "plugins/local/setEnabled"
        )
}

fn committed_bytes(root: &Path, commit: &str, path: &str) -> Result<Vec<u8>, Risk> {
    let spec = format!("{commit}:{path}");
    let size = Call::new(root, Index::User)
        .args(["cat-file", "-s", &spec])
        .max_output(128)
        .run()
        .map_err(|_| Risk::WorktreeMove)?;
    let size = std::str::from_utf8(&size)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or(Risk::WorktreeMove)?;
    if size > MAX_CONFIG_BYTES {
        return Err(Risk::Uncheckable);
    }
    Call::new(root, Index::User)
        .args(["cat-file", "blob", &spec])
        .max_output(usize::try_from(MAX_CONFIG_BYTES).unwrap_or(usize::MAX - 1) + 1)
        .run()
        .map_err(|_| Risk::WorktreeMove)
}

/// Before Vibe checks out a new branch, inspect its project MCP config and plugin definitions.
/// This reads the immutable commit Vibe will check out; edits made after
/// checkout still need a Vibe-side admission policy to eliminate the remaining race.
pub(crate) async fn check_checkout(root: PathBuf, commit: String) -> Result<(), CoreError> {
    let risk = crate::worktrees::blocking(move || {
        let tree = Call::new(&root, Index::User)
            .args(["ls-tree", "-r", "-z", "--name-only", &commit, "--", ".vibe"])
            .max_output(1024 * 1024)
            .run();
        let Ok(tree) = tree else {
            return Some(Risk::WorktreeMove);
        };
        for path in tree.split(|byte| *byte == 0) {
            let config = path == b".vibe/config.toml";
            let plugin = path.starts_with(b".vibe/plugins/") && path.ends_with(b"/mcp.json");
            if !config && !plugin {
                continue;
            }
            let Ok(path) = std::str::from_utf8(path) else {
                return Some(Risk::WorktreeMove);
            };
            let bytes = match committed_bytes(&root, &commit, path) {
                Ok(bytes) => bytes,
                Err(risk) => return Some(risk),
            };
            let risk = if config {
                match std::str::from_utf8(&bytes) {
                    Ok(text) => toml_risk(text),
                    Err(_) => Some(Risk::Uncheckable),
                }
            } else {
                let name = path
                    .strip_prefix(".vibe/plugins/")
                    .and_then(|rest| rest.strip_suffix("/mcp.json"))
                    .unwrap_or_default();
                if name.is_empty() || name.contains('/') {
                    continue;
                }
                plugin_document_risk(&bytes, name.to_owned(), root.join(path))
            };
            if risk.is_some() {
                return risk;
            }
        }
        None
    })
    .await?;
    risk.map_or(Ok(()), |risk| Err(refusal(risk, "worktrees/start")))
}

fn folders(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Result<(Vec<PathBuf>, Vec<String>), Risk> {
    // Lobby MCP calls can use Vibe's process cwd without naming a session.
    let mut roots = HashSet::from([shared.default_cwd.clone()]);
    if let Some(root) = params.get("projectRoot").and_then(Value::as_str) {
        roots.insert(absolute(root, &shared.default_cwd));
    }
    if matches!(
        method,
        "session/start" | "session/continue" | "session/resume" | "session/fork"
    ) {
        if let Some(target) =
            worktree_target(params.pointer("/agentConfig/worktree"), &shared.default_cwd)?
        {
            roots.insert(target);
        }
        let requested = params
            .pointer("/agentConfig/cwd")
            .or_else(|| params.pointer("/agentConfig/workdir"))
            .and_then(Value::as_str);
        if let Some(folder) = requested {
            roots.insert(absolute(folder, &shared.default_cwd));
        }
        add_workspace_roots(
            &mut roots,
            params.pointer("/agentConfig/workspaceRoots"),
            &shared.default_cwd,
        )?;
    }
    let mut unresolved = Vec::new();
    let session = params
        .get("sourceSessionId")
        .or_else(|| params.get("sessionId"))
        .and_then(Value::as_str);
    if let Some(session) = session {
        if let Some(folder) = shared.names.cwd_of(&shared.store, session) {
            roots.insert(absolute(&folder, &shared.default_cwd));
            if let Some(view) = shared.store.view(session) {
                add_workspace_roots(
                    &mut roots,
                    view.state.pointer("/session/workspaceRoots"),
                    &shared.default_cwd,
                )?;
            }
        } else if must_read_unknown_session(method, params) {
            unresolved.push(session.to_owned());
        }
    } else if method == "session/continue"
        && params
            .pointer("/agentConfig/cwd")
            .or_else(|| params.pointer("/agentConfig/workdir"))
            .and_then(Value::as_str)
            .is_none()
    {
        return Err(Risk::Uncheckable);
    }
    for open in shared.channels.open_channels() {
        let Some(folder) = shared.names.cwd_of(&shared.store, &open.session_id) else {
            unresolved.push(open.session_id.clone());
            continue;
        };
        roots.insert(absolute(&folder, &shared.default_cwd));
        if let Some(view) = shared.store.view(&open.session_id) {
            add_workspace_roots(
                &mut roots,
                view.state.pointer("/session/workspaceRoots"),
                &shared.default_cwd,
            )?;
        }
    }
    Ok((roots.into_iter().collect(), unresolved))
}

async fn checked_roots(
    shared: &Shared,
    method: &str,
    params: &Value,
) -> Result<Vec<PathBuf>, Risk> {
    let (mut roots, unresolved) = folders(shared, method, params)?;
    // A folder the request names is already among the roots (`folders`); it is the folder Vibe uses
    // when the stored chat has none (older imported chats keep `cwd: null`).
    let requested_folder = params
        .pointer("/agentConfig/cwd")
        .or_else(|| params.pointer("/agentConfig/workdir"))
        .and_then(Value::as_str)
        .is_some_and(|folder| !folder.is_empty());
    for id in unresolved {
        // A stored chat may be resumed before a sidebar listing has populated the name cache.
        // session/read only loads saved state; it does not open a chat or materialize MCP.
        let reply = shared
            .gate
            .call("session/read", serde_json::json!({"sessionId": id}), None)
            .await
            .map_err(|_| Risk::Uncheckable)?;
        let session = reply
            .result
            .pointer("/state/session")
            .ok_or(Risk::Uncheckable)?;
        let stored = session
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.is_empty());
        match stored {
            Some(cwd) => roots.push(absolute(cwd, &shared.default_cwd)),
            // No stored folder: the requested one (already checked) is the chat's folder.
            None if requested_folder => {}
            // Neither a stored nor a requested folder can be checked: refuse.
            None => return Err(Risk::Uncheckable),
        }
        let mut extra = HashSet::new();
        add_workspace_roots(
            &mut extra,
            session.get("workspaceRoots"),
            &shared.default_cwd,
        )?;
        roots.extend(extra);
    }
    Ok(roots)
}

fn refusal(risk: Risk, method: &str) -> CoreError {
    let (message, code) = match risk {
        Risk::Command => (
            vw_vocabulary::texts::PROJECT_COMMAND_SERVER_BLOCKED.to_owned(),
            "projectCommandServerBlocked",
        ),
        Risk::PluginCommand(name, path) => (
            format!(
                "{} Plugin: {name:?}. File: {:?}.",
                vw_vocabulary::texts::PROJECT_PLUGIN_COMMAND_SERVER_BLOCKED,
                path.display().to_string()
            ),
            "projectPluginCommandServerBlocked",
        ),
        Risk::Uncheckable => (
            vw_vocabulary::texts::PROJECT_COMMAND_SERVER_UNCHECKABLE.to_owned(),
            "projectCommandServerUncheckable",
        ),
        Risk::WorktreeMove => (
            vw_vocabulary::texts::PROJECT_COMMAND_SERVER_WORKTREE_MOVE.to_owned(),
            "projectCommandServerWorktreeMove",
        ),
    };
    let mut error = CoreError::new(ErrorCode::Denied, message).with_method(method);
    error.server_code = Some(code.to_owned());
    error
}

/// Refuse a request before Vibe can open, reload or converge a project command. Reads needed to close or inspect a
/// chat remain available. The error reaches both the window and the agent through the core's ordinary refusal path.
pub(crate) async fn admit(shared: &Shared, method: &str, params: &Value) -> Result<(), CoreError> {
    if matches!(
        method,
        "session/list" | "session/read" | "session/delete" | "session/archive" | "turn/interrupt"
    ) {
        return Ok(());
    }
    let roots = checked_roots(shared, method, params)
        .await
        .map_err(|risk| refusal(risk, method))?;
    let risk = crate::worktrees::blocking(move || roots.iter().find_map(|root| project_risk(root)))
        .await?;
    risk.map_or(Ok(()), |risk| Err(refusal(risk, method)))
}

/// Internal connector convergence has no direct caller to receive a denial; publish its reason.
pub(crate) async fn admit_refresh(shared: &Shared, method: &str, params: &Value) -> bool {
    let Err(error) = admit(shared, method, params).await else {
        return true;
    };
    tracing::warn!(method, reason = %error.message, "connectors: project MCP refresh refused");
    shared.emit(crate::event::CoreEvent::Refused {
        origin: crate::command::Origin::Ui,
        command: "connectors refresh".to_owned(),
        method: Some(method.to_owned()),
        code: error.code.as_str().to_owned(),
        reason: error.message,
    });
    false
}

/// An OFF transition cannot publish OFF while an already open chat may still have an enabled project command.
pub(crate) async fn before_off(shared: &Shared) -> Result<(), CoreError> {
    let roots = checked_roots(shared, "developerPermissions", &Value::Null)
        .await
        .map_err(|risk| refusal(risk, "developerPermissions"))?;
    let risk = crate::worktrees::blocking(move || roots.iter().find_map(|root| project_risk(root)))
        .await?;
    if risk.is_some() {
        let mut error = CoreError::new(
            ErrorCode::Denied,
            vw_vocabulary::texts::PROJECT_COMMAND_SERVER_OFF_BLOCKED,
        );
        error.server_code = Some("projectCommandServerOffBlocked".to_owned());
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_mcp_tests.rs"]
mod tests;
