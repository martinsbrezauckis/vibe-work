//! Forced discovery admission for shared MCP configuration changes.
//! Called under the Developer-permissions mutation lock by command pause and connector convergence.
//! Guarded launches exclude disabled stdio from discovery; unguarded launches defer it. OFF blocks enabled commands.

use std::{collections::HashSet, path::Path};

use crate::core::Shared;
use crate::mcp_keys_config::ConfigEditError;

fn config(home: &Path) -> Result<toml::Table, ConfigEditError> {
    crate::mcp_keys_config::read(&home.join("config.toml"))?
        .parse()
        .map_err(|_| ConfigEditError::Unsupported)
}

/// Whether forced discovery is unsafe for this launch. Validate both persisted TOML forms.
pub(crate) fn blocked_command(
    home: &Path,
    enabled: bool,
    guarded: bool,
) -> Result<bool, ConfigEditError> {
    let doc = config(home)?;
    let rows = crate::mcp_rows::read_rows(&doc)?;
    let disabled_stdio = rows.iter().any(|row| {
        row.get("transport").and_then(toml::Value::as_str) == Some("stdio")
            && row.get("disabled").and_then(toml::Value::as_bool) == Some(true)
    });
    if disabled_stdio {
        // Preserve the former deferred path's strict name checks: a disabled row
        // must remain distinguishable from every enabled row after Vibe normalizes names.
        let mut seen = HashSet::new();
        for row in &rows {
            let name = row
                .get("name")
                .and_then(toml::Value::as_str)
                .ok_or(ConfigEditError::Unsupported)?;
            let normalized = crate::command_servers_config::vibe_server_name(name);
            if normalized.is_empty() || !seen.insert(normalized) {
                return Err(ConfigEditError::Unsupported);
            }
        }
    }
    Ok((disabled_stdio && !guarded)
        || rows.iter().any(|row| {
            row.get("transport").and_then(toml::Value::as_str) == Some("stdio")
                && !enabled
                && row.get("disabled").and_then(toml::Value::as_bool) != Some(true)
        }))
}

/// Read the capability of the running process, including after a deferred start or restart.
pub(crate) fn guard_installed(shared: &Shared) -> bool {
    #[cfg(test)]
    if shared
        .mock_mcp_refresh_guard
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return true;
    }
    shared.supervisor.mcp_refresh_guard_installed()
}

/// Shared refreshes include enabled servers without discovering or re-enabling paused command rows.
pub(crate) fn admit_shared(shared: &Shared) -> bool {
    crate::command_pause::admit_internal_refresh(shared, None)
}

#[cfg(test)]
mod tests {
    use super::blocked_command;

    #[test]
    fn disabled_stdio_needs_the_guard_in_both_forms_and_switch_states() {
        let dir = tempfile::tempdir().unwrap();
        for text in [
            "mcp_servers=[{name='paused',transport='stdio',command='echo',disabled=true}]",
            "[[mcp_servers]]\nname='paused'\ntransport='stdio'\ncommand='echo'\ndisabled=true\n",
        ] {
            std::fs::write(dir.path().join("config.toml"), text).unwrap();
            for enabled in [false, true] {
                assert!(blocked_command(dir.path(), enabled, false).unwrap());
                assert!(!blocked_command(dir.path(), enabled, true).unwrap());
            }
        }
    }

    #[test]
    fn enabled_stdio_stays_blocked_while_off_with_either_launcher() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "mcp_servers=[{name='active',transport='stdio',command='echo'}]",
        )
        .unwrap();
        for guarded in [false, true] {
            assert!(blocked_command(dir.path(), false, guarded).unwrap());
            assert!(!blocked_command(dir.path(), true, guarded).unwrap());
        }
    }

    #[test]
    fn missing_empty_http_and_malformed_inputs_keep_their_admission_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for guarded in [false, true] {
            for enabled in [false, true] {
                assert!(!blocked_command(dir.path(), enabled, guarded).unwrap());
            }
        }
        for text in [
            "",
            "mcp_servers=[{name='web',transport='http',disabled=true}]",
        ] {
            std::fs::write(&path, text).unwrap();
            for guarded in [false, true] {
                for enabled in [false, true] {
                    assert!(!blocked_command(dir.path(), enabled, guarded).unwrap());
                }
            }
        }
        for text in [
            "mcp_servers=[42]",
            "mcp_servers=[{name='x',transport='stdio',command='echo',disabled='true'}]",
        ] {
            std::fs::write(&path, text).unwrap();
            for guarded in [false, true] {
                for enabled in [false, true] {
                    assert!(blocked_command(dir.path(), enabled, guarded).is_err());
                }
            }
        }
    }
}
