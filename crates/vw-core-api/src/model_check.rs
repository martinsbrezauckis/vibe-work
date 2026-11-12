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
//! Invariants: no `[[models]]` entry but Vibe Work's exact facts entry is ever removed
//! or rewritten, and none is added for an alias the person has an entry of their own for; nothing
//! changes while Vibe is still loading Mistral's lists (`awaitingExperimentModel`); a file that changed while it was
//! read is left alone (the next launch tries again).

use std::path::Path;

#[path = "model_check_write.rs"]
mod write;
use std::sync::Arc;
use write::replace;

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

async fn lobby(shared: &Shared) -> Result<Value, CoreError> {
    shared
        .gate
        .call("config/read", json!({}), None)
        .await
        .map(|reply| reply.result)
        .map_err(|error| CoreError::from_gate("config/read", &error))
}

/// Reads the lobby as Mistral sends it: Vibe Work's facts entry is taken out for the read and put back after, so
/// Mistral's own values can win. Returns the read and the file text without the facts entry. The record of our entry
/// (`model_facts::record`) is cleared before the entry leaves the file and restored only if the entry is put back.
async fn bare_lobby(
    shared: &Shared,
    home: &Path,
    path: &Path,
    account_models_available: bool,
) -> Result<(Value, String), CoreError> {
    let original = crate::default_model::read(path)?;
    let written = crate::model_facts::recorded(home);
    let Some(bare) = crate::model_facts::without(&original, written.as_ref()) else {
        if written.is_some() {
            // An absent or changed entry is no longer ours: discard the stale record.
            crate::model_facts::record(home, None)?;
        }
        return Ok((lobby(shared).await?, original));
    };
    crate::model_facts::record(home, None)?;
    if !replace(path, &original, &bare).await? {
        crate::model_facts::record(home, written.as_ref())?;
        return Ok((lobby(shared).await?, original));
    }
    let read = lobby(shared).await;
    // Without a usable vault key an empty account list does not establish that a model was retired.
    // Keep the saved facts through a locked update, including a file Vibe rewrote without our comment.
    // An authenticated empty list remains authoritative (e.g. experiments disabled).
    if read.is_err()
        || read.as_ref().is_ok_and(|r| {
            r["config"]["awaitingExperimentModel"] == true
                || (!account_models_available
                    && !r["config"]["models"].as_array().is_some_and(|models| {
                        models.iter().any(|model| {
                            matches!(model["source"].as_str(), Some("growthbook" | "catalog"))
                        })
                    }))
        })
    {
        if replace(path, &bare, &original).await? {
            crate::model_facts::record(home, written.as_ref())?;
        }
        return read.map(|r| (r, original));
    }
    Ok((read?, bare))
}

/// Runs the launch check: `{gone}` (the alias of a default Vibe no longer offers, or `null`).
///
/// # Errors
/// A lobby read the gate or Vibe refuses, or a config file that cannot be safely read or replaced.
pub(crate) async fn check(shared: &Arc<Shared>) -> Result<Value, CoreError> {
    let home = &shared.instructions.own_home;
    let path = home.join("config.toml");
    let (mut reply, current) = bare_lobby(
        shared,
        home,
        &path,
        shared
            .config
            .keys
            .as_ref()
            .is_none_or(|keys| keys.speech_available()),
    )
    .await?;
    if reply["config"]["awaitingExperimentModel"] == true {
        return Ok(json!({"gone": null}));
    }
    // Nothing moves away from Vibe Work's own `large-4` (module header): Mistral's entry only gets its facts, and
    // only when the person has no entry of their own for its alias (theirs wins as it is).
    let mut next = current.clone();
    let mut written = None;
    if let Some(row) = mistral_large_4(&reply["config"]) {
        let target = row["alias"].as_str().unwrap_or_default();
        if !crate::model_facts::has_entry_for(&current, target) {
            (next, written) = crate::model_facts::with(&next, target, row);
        }
    }
    if next != current {
        if replace(&path, &current, &next).await? {
            crate::model_facts::record(home, written.as_ref())?;
        }
        reply = lobby(shared).await?;
    }
    let gone = crate::model_choices::unavailable_default_alias(&reply["config"]);
    Ok(json!({"gone": gone}))
}

#[cfg(all(test, unix))]
#[path = "model_check_tests.rs"]
mod tests;
