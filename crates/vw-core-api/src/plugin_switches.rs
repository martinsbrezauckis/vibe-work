//! Plugin on/off through Vibe's own switches (`disabled_plugins` / `enabled_plugins` in the user config layer).
//!
//! Vibe's config file is the one record of whether a plugin is on: Turn off / Turn on ask Vibe's
//! `plugins/local/setEnabled` on the core's own server connection (the gate keeps that method core-only), and the
//! Tools page reads the switches again on every read, so an edit made by hand shows at the next read. The plugin
//! folder stays where it is. Folders an earlier Vibe Work version turned off by moving them to `plugins-off` are
//! moved back once, after their name is written to `disabled_plugins`, so they stay off.
//!
//! Depends on: the core's server channels, the plugin approvals record and its pinned identity reader, `toml_edit`.
//! Called by: `market_controls` (Turn off / Turn on) and `plugin_approvals` (the one-time move before launch).

use std::path::Path;

use serde_json::{Value, json};
use toml_edit::{Array, DocumentMut, Item, Value as TomlValue};

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};

/// The bundled plugin's manifest name: it is part of Vibe Work and is never switched off.
pub(crate) const BUNDLED_PLUGIN: &str = "vw";

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

pub(crate) const LOCKED_TEXT: &str =
    "This plugin is turned off by a setting outside Vibe Work's Tools page. Change it there.";
pub(crate) const COLLISION_TEXT: &str =
    "Another plugin that is on uses the same name. Turn that one off first.";
pub(crate) const BUNDLED_TEXT: &str = "Vibe Work's own plugin stays on.";

fn refuse(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

/// The Vibe Work text for a refused `plugins/local/setEnabled`, from Vibe's refusal reason.
pub(crate) fn refusal(error: &vw_rpc::RpcError) -> CoreError {
    if let vw_rpc::RpcError::Remote { error, .. } = error {
        match error
            .data
            .as_option()
            .and_then(|data| data.get("reason"))
            .and_then(Value::as_str)
        {
            Some("locked") => return refuse(LOCKED_TEXT),
            Some("namespace_collision") => return refuse(COLLISION_TEXT),
            _ => {}
        }
    }
    CoreError::new(
        ErrorCode::Internal,
        "Vibe could not change the plugin's setting. Read the Tools page again.",
    )
}

impl Core {
    /// Writes Vibe's own switch for the plugin `name` in the user config layer (every scope: Vibe switches plugins
    /// by name). Vibe lists a project's own plugins only for a trusted project, so for a project plugin the project
    /// is trusted for this one read (never saved); the write still goes to the user layer only.
    pub(crate) async fn set_plugin_switch(
        &self,
        name: &str,
        enabled: bool,
        project: Option<&Path>,
        project_plugin: bool,
    ) -> Result<(), CoreError> {
        if name == BUNDLED_PLUGIN {
            return Err(refuse(BUNDLED_TEXT));
        }
        let trust = project.is_some() && project_plugin;
        let mut params = json!({"name": name, "enabled": enabled, "trustWorkspace": trust});
        if let Some(project) = project {
            params["projectRoot"] = json!(project);
        }
        crate::project_mcp::admit(&self.shared, "plugins/local/setEnabled", &params).await?;
        self.shared
            .channels
            .request("plugins/local/setEnabled", params)
            .await
            .map(drop)
            .map_err(|error| refusal(&error))
    }
}

/// Adds `name` to the top-level `disabled_plugins` of a Vibe config file (created when missing), keeping the rest
/// of the file as written. Adding a name that is already there changes nothing.
pub(crate) fn add_disabled(config: &Path, name: &str) -> Result<(), String> {
    let before = match std::fs::symlink_metadata(config) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Ok(meta) if meta.file_type().is_file() && meta.len() <= MAX_CONFIG_BYTES => {
            std::fs::read_to_string(config).map_err(|e| e.to_string())?
        }
        Ok(_) => return Err("the Vibe config is not a plain file".to_owned()),
        Err(error) => return Err(error.to_string()),
    };
    let mut document: DocumentMut = before
        .parse()
        .map_err(|_| "the Vibe config is not valid TOML".to_owned())?;
    let list = match document.get_mut("disabled_plugins") {
        None => {
            document["disabled_plugins"] = Item::Value(TomlValue::Array(Array::new()));
            document["disabled_plugins"]
                .as_array_mut()
                .ok_or("the new list could not be made")?
        }
        Some(item) => item
            .as_array_mut()
            .ok_or("disabled_plugins in the Vibe config is not a list")?,
    };
    if list.iter().any(|entry| entry.as_str() == Some(name)) {
        return Ok(());
    }
    list.push(name);
    let after = document.to_string();
    if after.parse::<toml::Table>().is_err() {
        return Err("the Vibe config could not be edited safely".to_owned());
    }
    vw_platform::replace_private(config, after.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::add_disabled;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vw-switches-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn add_disabled_keeps_the_file_and_adds_each_name_once() {
        let file = scratch("keep");
        std::fs::write(
            &file,
            "# mine\nactive_model = \"m\"\ndisabled_plugins = [\"a\"]\n",
        )
        .unwrap();
        add_disabled(&file, "b").unwrap();
        add_disabled(&file, "b").unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert!(text.starts_with("# mine\n"), "{text}");
        assert_eq!(table["active_model"].as_str(), Some("m"));
        assert_eq!(
            table["disabled_plugins"].as_array().unwrap().len(),
            2,
            "{text}"
        );
    }

    #[test]
    fn add_disabled_creates_the_list_and_refuses_a_wrong_type() {
        let file = scratch("create");
        add_disabled(&file, "x").unwrap();
        let table: toml::Table = std::fs::read_to_string(&file).unwrap().parse().unwrap();
        assert_eq!(table["disabled_plugins"][0].as_str(), Some("x"));
        std::fs::write(&file, "disabled_plugins = \"x\"\n").unwrap();
        assert!(add_disabled(&file, "y").is_err());
    }
}
