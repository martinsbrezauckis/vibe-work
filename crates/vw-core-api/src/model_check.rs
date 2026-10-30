//! The default model at launch (`checkDefaultModel`, the window runs it once Vibe is running):
//! - Vibe Work's own `large-4` entry stays as it is, and so does a default on it: that entry names the model Mistral
//!   serves (`mistral-large-4`). Mistral's own Large 4 entry, when Mistral's lists offer it and it is not marked
//!   preview, gets the facts it does not report from Vibe Work's table (`model_facts`), so its window and prices show;
//!   nothing moves to it. Its alias comes from the account's model list, which can name an id Mistral no longer serves.
//! - A Settings default that Vibe no longer offers (Mistral retired it, or the configuration lost it) is named in the
//!   answer (`gone`), so the window asks which model to use from now on.
//!
//! Mistral's entry: a model row whose `name` is the Large 4 API model (`mistral-large-4`, `mistral-large-4-0`), whose
//! alias is not Vibe Work's `large-4`, that comes from Mistral's own lists (Vibe's `growthbook` or `catalog` layer,
//! `app_server/config.py` `ModelSource`), and whose display name does not say "Preview" (the only preview mark Vibe
//! passes on).
//!
//! Depends on: the gate (`config/read`), `default_model` (bounded config read), `model_facts`, `model_choices`, the
//! private atomic writer. Called by: `dispatch` (`checkDefaultModel`).
//! Invariants: no `[[models]]` entry but Vibe Work's own marked facts entry is ever removed or rewritten; nothing
//! changes while Vibe is still loading Mistral's lists (`awaitingExperimentModel`); a file that changed while it was
//! read is left alone (the next launch tries again).

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// The alias of Vibe Work's own Large 4 entry (`vw_host::default_model`).
const OWN_ALIAS: &str = "large-4";

/// API names of Mistral Large 4.
const LARGE_4: &[&str] = &["mistral-large-4", "mistral-large-4-0"];

/// Mistral's own Large 4 row in a config view (`{models: [...]}`) read without Vibe Work's facts entry, when it is
/// offered and not marked preview.
fn mistral_large_4(config: &Value) -> Option<&Value> {
    config["models"].as_array()?.iter().find(|model| {
        let alias = model["alias"].as_str();
        model["name"]
            .as_str()
            .is_some_and(|name| LARGE_4.contains(&name))
            && alias.is_some_and(|alias| alias != OWN_ALIAS)
            && matches!(model["source"].as_str(), Some("growthbook" | "catalog"))
            && !model["displayName"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains("preview")
    })
}

fn failed(message: &'static str) -> CoreError {
    CoreError::new(ErrorCode::Internal, message)
}

/// Replaces `path` with `after` when it still holds `before`; false when another writer changed it meanwhile.
fn replace(path: &Path, before: &str, after: &str) -> Result<bool, CoreError> {
    if after.parse::<toml::Table>().is_err() {
        return Err(failed("Vibe Work's model config cannot be edited safely"));
    }
    if crate::default_model::read(path)? != before {
        return Ok(false);
    }
    vw_platform::replace_private(path, after.as_bytes())
        .map_err(|_| failed("Vibe Work's model config could not be saved"))?;
    Ok(true)
}

async fn lobby(shared: &Shared) -> Result<Value, CoreError> {
    shared
        .gate
        .call("config/read", json!({}), None)
        .await
        .map(|reply| reply.result)
        .map_err(|error| CoreError::from_gate("config/read", &error))
}

/// Reads the lobby as Mistral sends it: Vibe Work's facts entry is taken out for the read and put back after, so
/// Mistral's own values can win. Returns the read and the file text without the facts entry.
async fn bare_lobby(shared: &Shared, path: &Path) -> Result<(Value, String), CoreError> {
    let original = crate::default_model::read(path)?;
    let Some(bare) = crate::model_facts::without(&original) else {
        return Ok((lobby(shared).await?, original));
    };
    if !replace(path, &original, &bare)? {
        return Ok((lobby(shared).await?, original));
    }
    let read = lobby(shared).await;
    if read.is_err()
        || read
            .as_ref()
            .is_ok_and(|r| r["config"]["awaitingExperimentModel"] == true)
    {
        replace(path, &bare, &original)?;
        return read.map(|r| (r, original));
    }
    Ok((read?, bare))
}

/// Runs the launch check: `{gone}` (the alias of a default Vibe no longer offers, or `null`).
///
/// # Errors
/// A lobby read the gate or Vibe refuses, or a config file that cannot be safely read or replaced.
pub(crate) async fn check(shared: &Arc<Shared>) -> Result<Value, CoreError> {
    let path = shared.instructions.own_home.join("config.toml");
    let (mut reply, current) = bare_lobby(shared, &path).await?;
    if reply["config"]["awaitingExperimentModel"] == true {
        return Ok(json!({"gone": null}));
    }
    // Nothing moves away from Vibe Work's own `large-4` (module header): Mistral's entry only gets its facts.
    let mut next = current.clone();
    if let Some(row) = mistral_large_4(&reply["config"]) {
        let target = row["alias"].as_str().unwrap_or_default().to_owned();
        next = crate::model_facts::with(&next, &target, row);
    }
    if next != current {
        replace(&path, &current, &next)?;
        reply = lobby(shared).await?;
    }
    let gone = crate::model_choices::unavailable_default_alias(&reply["config"]);
    Ok(json!({"gone": gone}))
}
