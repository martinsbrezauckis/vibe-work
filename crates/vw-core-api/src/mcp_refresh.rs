//! Forced discovery admission and deferred convergence for shared MCP configuration changes.
//! Called under the Developer-permissions mutation lock by command pause and connector convergence.
//! A disabled stdio row must never reach forced discovery, even while Developer permissions is on.

use std::collections::HashSet;
use std::path::Path;

use crate::core::Shared;
use crate::mcp_keys_config::ConfigEditError;

fn config(home: &Path) -> Result<toml::Table, ConfigEditError> {
    crate::mcp_keys_config::read(&home.join("config.toml"))?
        .parse()
        .map_err(|_| ConfigEditError::Unsupported)
}

/// OFF blocks every command row; ON still blocks every disabled command row. Both TOML forms are validated.
pub(crate) fn blocked_command(home: &Path, enabled: bool) -> Result<bool, ConfigEditError> {
    let doc = config(home)?;
    Ok(crate::mcp_rows::read_rows(&doc)?.iter().any(|row| {
        row.get("transport").and_then(toml::Value::as_str) == Some("stdio")
            && (!enabled || row.get("disabled").and_then(toml::Value::as_bool) == Some(true))
    }))
}

fn deferred_targets(home: &Path) -> Result<Option<Vec<String>>, ConfigEditError> {
    let doc = config(home)?;
    let rows = crate::mcp_rows::read_rows(&doc)?;
    if !rows.iter().any(|row| {
        row.get("transport").and_then(toml::Value::as_str) == Some("stdio")
            && row.get("disabled").and_then(toml::Value::as_bool) == Some(true)
    }) {
        return Ok(None);
    }
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for row in rows {
        let name = row
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or(ConfigEditError::Unsupported)?;
        let normalized = crate::command_servers_config::vibe_server_name(name);
        if normalized.is_empty() || !seen.insert(normalized) {
            return Err(ConfigEditError::Unsupported);
        }
        if row.get("disabled").and_then(toml::Value::as_bool) != Some(true) {
            targets.push(name.to_owned());
        }
    }
    Ok(Some(targets))
}

/// Shared changes with disabled command rows use the switch's before-work reload and targeted toggles.
/// Running turns finish. A later apply checks current rows again, so a subsequent person-disable wins.
pub(crate) fn admit_shared(shared: &Shared) -> bool {
    if shared.developer_permissions.enabled() {
        match deferred_targets(&shared.instructions.own_home) {
            Ok(Some(targets)) => {
                shared
                    .developer_permissions
                    .remember_changed_servers(targets);
                shared.channels.bump_switch_generation();
                return false;
            }
            Ok(None) => {}
            Err(_) => {
                crate::command_pause::notify_internal_refresh_withheld(
                    shared,
                    None,
                    crate::developer_permissions::RefreshWithheldCause::InspectFailed,
                );
                return false;
            }
        }
    }
    crate::command_pause::admit_internal_refresh(shared, None)
}
