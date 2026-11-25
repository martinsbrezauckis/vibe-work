//! Apply enabled servers queued by the switch or a shared config change in an idle session.
//! Called by `switch_apply` after config/reload, under both the chat send lock and the mutation lock.
//! Pinned Vibe 3.1.4's per-server toggle converges without forced discovery; a broad refresh would also run
//! disabled stdio rows. Only core-owned config changes supply names, and the current config is checked again
//! before enabling them. No agent or UI parameter selects these targets or bypasses the ordinary toggle gate.

use std::collections::HashSet;
use std::path::Path;

use serde_json::json;
use tracing::warn;

use crate::core::Shared;
use crate::mcp_keys_config::ConfigEditError;

fn enabled_targets(home: &Path, resumed: &[String]) -> Result<Vec<String>, ConfigEditError> {
    let text = crate::mcp_keys_config::read(&home.join("config.toml"))?;
    let doc = text
        .parse::<toml::Table>()
        .map_err(|_| ConfigEditError::Unsupported)?;
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for row in crate::mcp_rows::read_rows(&doc)? {
        let name = row
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or(ConfigEditError::Unsupported)?;
        let normalized = crate::command_servers_config::vibe_server_name(name);
        if normalized.is_empty() || !seen.insert(normalized) {
            return Err(ConfigEditError::Unsupported);
        }
        if resumed.iter().any(|resumed| resumed == name)
            && row.get("disabled").and_then(toml::Value::as_bool) != Some(true)
        {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

pub(super) async fn apply(shared: &Shared, id: &str) -> bool {
    if !shared.developer_permissions.enabled() {
        return false;
    }
    let names = match enabled_targets(
        &shared.instructions.own_home,
        &shared.developer_permissions.resumed_commands(),
    ) {
        Ok(names) => names,
        Err(error) => {
            warn!(session_id = id, %error, "developer permissions: resumed servers could not be inspected");
            return false;
        }
    };
    for name in names {
        let method = "mcp_catalog/toggle";
        let params = json!({"sessionId": id, "name": name, "source": "server", "disabled": false});
        if !crate::project_mcp::admit_refresh(shared, method, &params).await {
            return false;
        }
        if let Err(error) = shared.gate.call(method, params, None).await {
            warn!(session_id = id, %error, "developer permissions: a resumed server did not apply");
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_selection_is_strict_and_never_selects_person_disabled_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let resumed = vec!["resumed".to_owned(), "off-again".to_owned()];
        for config in [
            r#"mcp_servers = [{name="resumed",transport="stdio",command="echo"},{name="off-again",transport="stdio",command="echo",disabled=true},{name="person-disabled",transport="stdio",command="echo",disabled=true},{name="other",transport="stdio",command="echo"}]"#,
            "[[mcp_servers]]\nname='resumed'\ntransport='stdio'\ncommand='echo'\n[[mcp_servers]]\nname='off-again'\ntransport='stdio'\ncommand='echo'\ndisabled=true\n",
        ] {
            std::fs::write(&path, config).unwrap();
            assert_eq!(enabled_targets(dir.path(), &resumed).unwrap(), ["resumed"]);
        }
        for config in [
            "mcp_servers = 'invalid'",
            r#"mcp_servers = [{name="resumed",transport="stdio",command="echo",disabled="false"}]"#,
            r#"mcp_servers = [{name="resumed",transport="stdio",command="echo"},{name="_resumed_",transport="stdio",command="echo",disabled=true}]"#,
        ] {
            std::fs::write(&path, config).unwrap();
            assert!(enabled_targets(dir.path(), &resumed).is_err(), "{config}");
        }
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            enabled_targets(dir.path(), &resumed).unwrap(),
            Vec::<String>::new()
        );
        std::fs::write(&path, "").unwrap();
        assert_eq!(
            enabled_targets(dir.path(), &resumed).unwrap(),
            Vec::<String>::new()
        );
    }
}
