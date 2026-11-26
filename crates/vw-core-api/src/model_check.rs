//! The default model at launch (`checkDefaultModel`, the window runs it once Vibe is running):
//! - A catalog row keeps Mistral's model facts. An older experiment Large 4 entry may receive missing facts from
//!   `model_facts`; the app does not add a local override to a catalog model.
//! - A Settings default that Vibe no longer offers (Mistral retired it, or the configuration lost it) is named in the
//!   answer (`gone`), so the window asks which model to use from now on.
//! - The retired app seed's `large-4` alias is updated to one catalog Large 4 id after Vibe loads the catalog,
//!   preserving the person's chosen model even when a different model is recommended; while the catalog is not
//!   loaded (no key yet, offline) to Vibe's own built-in Large 4, so an upgrade never asks for a model that is there.
//! - A chat's saved choice of that alias (`model_choices`, model-choices.json) is checked against its own model list
//!   at reopen or Send. The lobby has no project folder, so it cannot decide which provider a chat chose.
//!
//! Mistral's entry: a model row whose `name` is the Large 4 API model (`mistral-large-4`, `mistral-large-4-0`), whose
//! alias is not Vibe Work's `large-4`, that comes from an older `growthbook` layer, and whose display name does not
//! say "Preview" (the only preview mark Vibe passes on).
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
use toml_edit::{DocumentMut, value};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// The alias of Vibe Work's own Large 4 entry (`vw_host::default_model`).
pub(crate) const OWN_ALIAS: &str = "large-4";

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
            && model["source"] == "growthbook"
            && !model["displayName"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains("preview")
    })
}

/// The old app default named Large 4 by its local alias. After that exact app row is retired,
/// preserve the selected model by resolving one unambiguous catalog Large 4 row, or without catalog rows Vibe's one
/// built-in Large 4 row (not a row of the person's file). A person's own `large-4` row and a choice between multiple
/// Large 4 ids are left alone.
fn legacy_catalog_choice(text: &str, config: &Value) -> Option<String> {
    let settings: toml::Table = text.parse().ok()?;
    if settings.get("active_model")?.as_str()? != OWN_ALIAS
        || settings
            .get("enable_model_catalog")
            .or_else(|| settings.get("experimental_enable_model_catalog"))
            .and_then(toml::Value::as_bool)
            != Some(true)
        || crate::model_choices::unavailable_default_alias(config).as_deref() != Some(OWN_ALIAS)
    {
        return None;
    }
    // The person's own rows (any alias or name in the file): never taken for Vibe's built-in row.
    let mut own = Vec::new();
    if let Some(rows) = settings.get("models") {
        for row in rows.as_array()? {
            for key in ["alias", "name"] {
                if let Some(text) = row.get(key).and_then(toml::Value::as_str) {
                    own.push(text.to_owned());
                }
            }
        }
        if own.iter().any(|text| text == OWN_ALIAS) {
            return None;
        }
    }
    let only = |mut rows: Vec<String>| (rows.len() == 1).then(|| rows.remove(0));
    let catalog: Vec<String> = large_4_rows(config, "catalog")
        .map(|(alias, _)| alias)
        .collect();
    if !catalog.is_empty() {
        return only(catalog);
    }
    // Mistral's list is not loaded (no key yet, offline): Vibe's own built-in Large 4 is the same model.
    only(
        large_4_rows(config, "local")
            .filter(|(alias, name)| !own.contains(alias) && !own.contains(name))
            .map(|(alias, _)| alias)
            .collect(),
    )
}

/// `(alias, name)` of the offered Large 4 rows from `source` (`catalog`, `local`), not preview, not our old alias.
fn large_4_rows<'a>(
    config: &'a Value,
    source: &'a str,
) -> impl Iterator<Item = (String, String)> + 'a {
    config["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(move |row| {
            let name = row["name"].as_str()?;
            let alias = row["alias"].as_str()?;
            (row["source"] == source
                && LARGE_4.contains(&name)
                && !alias.is_empty()
                && alias != OWN_ALIAS
                && !row["displayName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains("preview"))
            .then(|| (alias.to_owned(), name.to_owned()))
        })
}

/// The one offered catalog Large 4 row in a config view (`{models: [...]}`), not preview and not our old alias; `None`
/// when the catalog offers none or more than one. Chats saved on our old alias resolve to it (`model_choices`).
pub(crate) fn catalog_large_4(config: &Value) -> Option<&Value> {
    let mut rows = large_4_rows(config, "catalog");
    let (alias, _) = rows.next()?;
    if rows.next().is_some() {
        return None;
    }
    config["models"]
        .as_array()?
        .iter()
        .find(|row| row["source"] == "catalog" && row["alias"] == alias.as_str())
}

fn with_active_model(text: &str, alias: &str) -> Option<String> {
    let mut config: DocumentMut = text.parse().ok()?;
    let decor = config.get("active_model")?.as_value()?.decor().clone();
    config["active_model"] = value(alias);
    *config["active_model"].as_value_mut()?.decor_mut() = decor;
    Some(config.to_string())
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

fn has_account_models(config: &Value) -> bool {
    config["models"].as_array().is_some_and(|models| {
        models
            .iter()
            .any(|model| matches!(model["source"].as_str(), Some("growthbook" | "catalog")))
    })
}

/// Missing managed-account rows cannot establish retirement, even with a usable vault key:
/// Vibe may have failed to fetch the catalog through the network or proxy. A core without
/// the managed key service retains its local-only config behavior.
pub(crate) fn account_models_unavailable(shared: &Shared, config: &Value) -> bool {
    account_models_unavailable_reason(shared, config).is_some()
}

pub(crate) enum AccountModelsUnavailableReason {
    KeyNotUsable,
    NotFetched,
}

/// Distinguishes a missing credential from an unfetched list without inspecting error text.
pub(crate) fn account_models_unavailable_reason(
    shared: &Shared,
    config: &Value,
) -> Option<AccountModelsUnavailableReason> {
    let keys = shared.config.keys.as_ref()?;
    if has_account_models(config) {
        return None;
    }
    Some(if keys.speech_available() {
        AccountModelsUnavailableReason::NotFetched
    } else {
        AccountModelsUnavailableReason::KeyNotUsable
    })
}

/// Reads the lobby as Mistral sends it: Vibe Work's facts entry is taken out for the read and put back after, so
/// Mistral's own values can win. Returns the read and the file text without the facts entry. The record of our entry
/// (`model_facts::record`) is cleared before the entry leaves the file and restored only if the entry is put back.
async fn bare_lobby(
    shared: &Shared,
    home: &Path,
    path: &Path,
    allow_missing_account_models: bool,
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
    // A managed account's absent list is inconclusive, whether its key is locked or its fetch failed.
    // Keep saved facts, including a file Vibe rewrote without our comment, until account rows arrive.
    if read.is_err()
        || read.as_ref().is_ok_and(|r| {
            r["config"]["awaitingExperimentModel"] == true
                || (!allow_missing_account_models && !has_account_models(&r["config"]))
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
    let allow_missing_account_models = shared.config.keys.is_none();
    let (mut reply, mut current) =
        bare_lobby(shared, home, &path, allow_missing_account_models).await?;
    if reply["config"]["awaitingExperimentModel"] == true {
        return Ok(json!({"gone": null}));
    }
    if let Some(alias) = legacy_catalog_choice(&current, &reply["config"])
        && let Some(resolved) = with_active_model(&current, &alias)
        && replace(&path, &current, &resolved).await?
    {
        current = resolved;
        reply = lobby(shared).await?;
    }
    // A historical experiment entry only gets missing facts when the person has no row of
    // their own for its alias. Catalog rows retain Mistral's facts as reported.
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
    // Key usability does not prove that Vibe fetched the account's list.
    let gone = if account_models_unavailable(shared, &reply["config"]) {
        None
    } else {
        crate::model_choices::unavailable_default_alias(&reply["config"])
    };
    Ok(json!({"gone": gone}))
}

#[cfg(all(test, unix))]
#[path = "model_check_tests.rs"]
mod tests;
