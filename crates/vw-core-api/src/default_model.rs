//! Saves the model used by future sessions in Vibe Work's own `VIBE_HOME/config.toml`.
//!
//! Vibe 2.26.1 has no session-independent config write RPC: `config/write` and
//! `config/model/write` require an attached session and also update its pin. The
//! lobby can read config, so this module admits an offered alias through the
//! gated lobby read, then edits only the durable `active_model` key. Existing
//! session channels and their config snapshots are never touched.
//!
//! Depends on: the core gate, `toml_edit`, and the private atomic file writer.
//! Called by: `dispatch` for `setDefaultModel`.

use std::path::Path;

use serde_json::{Value, json};
use toml_edit::{DocumentMut, Item, Value as TomlValue};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

fn refused(message: &'static str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn failed(message: &'static str) -> CoreError {
    CoreError::new(ErrorCode::Internal, message)
}

fn offered(config: &Value, alias: &str) -> bool {
    config
        .pointer("/config/models")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models
                .iter()
                .any(|model| model.get("alias").and_then(Value::as_str) == Some(alias))
        })
}

fn read(path: &Path) -> Result<String, CoreError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| failed("Vibe Work's model config could not be read"))?;
    if metadata.len() > MAX_CONFIG_BYTES || !metadata.file_type().is_file() {
        return Err(failed("Vibe Work's model config cannot be edited safely"));
    }
    std::fs::read_to_string(path).map_err(|_| failed("Vibe Work's model config could not be read"))
}

fn write(path: &Path, alias: &str) -> Result<(), CoreError> {
    for _ in 0..2 {
        let before = read(path)?;
        let mut config: DocumentMut = before
            .parse()
            .map_err(|_| failed("Vibe Work's model config is not valid TOML"))?;
        if config
            .get("active_model")
            .and_then(Item::as_str)
            .is_some_and(|current| current == alias)
        {
            return Ok(());
        }
        if config.contains_key("active_model")
            && config.get("active_model").and_then(Item::as_str).is_none()
        {
            return Err(failed("Vibe Work's model config cannot be edited safely"));
        }
        let decor = config
            .get("active_model")
            .and_then(Item::as_value)
            .map(|old| old.decor().clone());
        let mut selected = TomlValue::from(alias);
        if let Some(decor) = decor {
            *selected.decor_mut() = decor;
        }
        config["active_model"] = Item::Value(selected);
        let after = config.to_string();
        if after.parse::<toml::Table>().is_err() {
            return Err(failed("Vibe Work's model config cannot be edited safely"));
        }
        if read(path)? != before {
            continue;
        }
        vw_platform::replace_private(path, after.as_bytes())
            .map_err(|_| failed("Vibe Work's model config could not be saved"))?;
        return Ok(());
    }
    Err(CoreError::new(
        ErrorCode::Stale,
        "Vibe Work's model config changed while saving; try again",
    ))
}

/// Checks the lobby's current model list, then saves the new-session model.
///
/// # Errors
/// A rejected or unavailable lobby read, an unoffered alias, or a config file
/// that cannot be safely read and atomically replaced.
pub(crate) async fn set(shared: &Shared, alias: &str) -> Result<(), CoreError> {
    if alias.is_empty() || alias.len() > 128 {
        return Err(refused("Choose a model offered by Vibe"));
    }
    let reply = shared
        .gate
        .call("config/read", json!({}), None)
        .await
        .map_err(|error| CoreError::from_gate("config/read", &error))?;
    if !offered(&reply.result, alias) {
        return Err(refused("Choose a model offered by Vibe"));
    }
    write(&shared.instructions.own_home.join("config.toml"), alias)
}
