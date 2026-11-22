//! Saved chat model/thinking choices in one record, replayed until Vibe pins the next turn.
//! Depends on the gated RPC, private atomic files and `sessions_dir`; called by sessions and restore.
//! Vibe 3.0.2's sessionOnly writes only memory. Replay before publishing runtime; failed model
//! restores alert without closing the chat. No user config or Vibe session files are edited here.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use vw_platform::sync::LockRecover;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

mod handoff;
pub(crate) use handoff::{forked, handed_off};
mod restore;
pub(crate) use restore::restore;

/// The alias of a Settings default Vibe no longer offers (its own validation warning), else `None`.
pub(crate) fn unavailable_default_alias(config: &Value) -> Option<String> {
    restore::unavailable_default(config).map(|choice| choice.alias)
}
mod records;
use records::Records;

mod bootstrap;
pub(crate) use bootstrap::prepare_builtin_models;

mod retired;

mod live;
pub(crate) use live::{admission, before, pick, publish, selected};

mod provider_recovery;
pub(crate) use provider_recovery::completed as provider_completed;

#[cfg(test)]
mod persistence_tests;
#[cfg(all(test, unix))]
mod retired_tests;
#[cfg(all(test, unix))]
mod runtime_tests;

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choice {
    alias: String,
    effort: String,
    // Keep a label even if the alias disappears.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    /// Provider identity is required to distinguish a project row from a catalog row with the same wire name.
    /// 0.1.15 records have no provider and stay ambiguous until the person selects a model again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    provider: String,
    /// Digest of the resolved provider route. Never persist the provider config or its headers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    provider_key: String,
}

impl Choice {
    fn matches(&self, other: &Self) -> bool {
        self.same_model(other) && self.effort == other.effort
    }

    fn same_model(&self, other: &Self) -> bool {
        self.alias == other.alias
            && (self.name.is_empty() || self.name == other.name)
            && (self.provider.is_empty() || self.provider == other.provider)
            && (self.provider_key.is_empty() || self.provider_key == other.provider_key)
    }

    fn label(&self) -> String {
        let name = if self.display_name.is_empty() {
            &self.alias
        } else {
            &self.display_name
        };
        if self.effort.is_empty() {
            name.clone()
        } else if self.effort == "off" {
            format!("{name} with thinking off")
        } else {
            let level = if self.effort == "max" {
                "maximum"
            } else {
                &self.effort
            };
            format!("{name} with {level} thinking")
        }
    }
}

/// Accepted chat picks and recovery defaults awaiting Vibe's durable turn pins.
pub(crate) struct Choices {
    records: Records,
    /// The app-owned proxy changes its loopback port on each start; its backend does not.
    mistral_proxy_route: Option<String>,
    handed_off: Mutex<std::collections::BTreeSet<(String, String)>>,
    /// Display names by alias from the model lists seen in this run ([`Self::learn`]).
    names: Mutex<BTreeMap<String, String>>,
    /// Latest rejected turn admitted for recovery in each chat; repeated notifications do not repeat the alert.
    rejected_turns: Mutex<BTreeMap<String, String>>,
    /// A bound pick whose cold, incomplete model list required a readiness refresh. Keep this
    /// across restore and the first Send so a failed replay cannot erase that pick.
    cold_replay: Mutex<BTreeMap<String, Choice>>,
}

/// Aliases whose display name is remembered (a model list holds a few dozen at most).
const MAX_NAMES: usize = 512;

impl Choices {
    pub(crate) fn load(dir: Option<&std::path::Path>) -> Self {
        Self {
            records: Records::load(dir),
            mistral_proxy_route: None,
            handed_off: Mutex::default(),
            names: Mutex::default(),
            rejected_turns: Mutex::default(),
            cold_replay: Mutex::default(),
        }
    }

    pub(crate) fn with_mistral_proxy_route(mut self, route: Option<String>) -> Self {
        self.mistral_proxy_route = route;
        self
    }

    /// Remembers the display name of every model in a runtime's or configuration's model list (`config.models`), so
    /// a model that later leaves Vibe's list can still be named ([`Self::named`]).
    pub(crate) fn learn(&self, runtime: &Value) {
        let Some(models) = runtime.pointer("/config/models").and_then(Value::as_array) else {
            return;
        };
        let mut names = self.names.lock_or_recover();
        for model in models {
            if let (Some(alias), Some(name)) = (
                model["alias"].as_str().filter(|a| !a.is_empty()),
                model["displayName"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty()),
            ) {
                if names.len() >= MAX_NAMES && !names.contains_key(alias) {
                    continue;
                }
                names.insert(alias.to_owned(), name.to_owned());
            }
        }
    }

    /// `choice` with a display name when it has none: from the last model list that listed its alias, else from a
    /// chat's saved choice of that alias. Unknown stays unnamed (the alias is never shown).
    pub(crate) fn named(&self, mut choice: Choice) -> Choice {
        if !choice.display_name.is_empty() {
            return choice;
        }
        let known = self
            .names
            .lock_or_recover()
            .get(&choice.alias)
            .cloned()
            .or_else(|| self.records.named(&choice.alias));
        if let Some(name) = known {
            choice.display_name = name;
        }
        choice
    }

    /// The model name reopening restores for `alias` (empty when none), in fallback/pending/remembered order:
    /// what Vibe's alias-only pin stands for in that chat ([`retired_row`]).
    // Only the unix-only retired tests call it; on Windows a plain cfg(test) leaves it dead and clippy -D warnings fails.
    #[cfg(all(test, unix))]
    pub(crate) fn saved_name(&self, id: &str, alias: &str) -> String {
        self.saved_identity(id, alias).0
    }

    /// Read both identity fields from one record snapshot; a concurrent choice update cannot mix them.
    pub(crate) fn saved_identity(&self, id: &str, alias: &str) -> (String, String) {
        self.records
            .get(id)
            .expected()
            .filter(|saved| saved.alias == alias)
            .map(|saved| (saved.name, saved.provider))
            .unwrap_or_default()
    }

    pub(crate) fn forget(&self, id: &str) {
        self.records.forget(id);
        self.rejected_turns.lock_or_recover().remove(id);
        self.cold_replay.lock_or_recover().remove(id);
        self.handed_off
            .lock_or_recover()
            .retain(|(old, new)| old != id && new != id);
    }

    fn mark_cold_replay(&self, id: &str, choice: Choice) {
        self.cold_replay.lock_or_recover().insert(id.into(), choice);
    }

    fn cold_replay_matches(&self, id: &str, choice: &Choice) -> bool {
        let mut cold = self.cold_replay.lock_or_recover();
        if cold.get(id).is_some_and(|saved| saved != choice) {
            cold.remove(id);
        }
        cold.get(id) == Some(choice)
    }

    fn clear_cold_replay(&self, id: &str) {
        self.cold_replay.lock_or_recover().remove(id);
    }

    /// Records accepted picks only; turns retire them after Vibe confirms its durable pins.
    #[cfg(test)]
    pub(crate) fn observe(
        &self,
        method: &str,
        params: &Value,
        result: &Value,
    ) -> Result<(), CoreError> {
        let Some(id) = params["sessionId"].as_str() else {
            return Ok(());
        };
        if method != "config/model/write" {
            return Ok(());
        }
        if result["modelChoiceFallback"] == true {
            return Ok(());
        }
        accepted(result)?;
        let choice = actual(result)
            .ok_or_else(|| CoreError::bad_result(method, "runtime.config.activeModel"))?;
        self.accept_pick(id, params, choice)
    }

    fn accept_pick(&self, id: &str, params: &Value, choice: Choice) -> Result<(), CoreError> {
        if params["modelAlias"]
            .as_str()
            .is_some_and(|alias| alias != choice.alias)
            || params["reasoningEffort"]
                .as_str()
                .is_some_and(|effort| effort != choice.effort)
        {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "This configuration did not apply the chat's model or thinking choice.",
            ));
        }
        self.records.update(id, |record| record.accepted(choice))
    }
}

/// The model row a chat saved on the retired `large-4` alias shows while it is not open: the config view's
/// (`{models}`) one catalog Large 4 row with the chat's own level (`retired`), the same model reopen and Send put
/// back. `name` is the model the chat's saved choice records for the alias ([`Choices::saved_name`]): a project's own
/// model under the alias is not Large 4. The saved provider must also prove the app's seed. `None` for any other
/// alias or model, or when the list does not settle it.
pub(crate) fn retired_row(
    config: &Value,
    alias: &str,
    effort: &str,
    name: &str,
    provider: &str,
) -> Option<Value> {
    let saved = Choice {
        alias: alias.into(),
        effort: effort.into(),
        display_name: String::new(),
        name: name.into(),
        provider: provider.into(),
        provider_key: String::new(),
    };
    let resolved = retired::resolve(&json!({"config":config}), &saved)?;
    let mut row = config["models"]
        .as_array()?
        .iter()
        .find(|row| row["alias"] == resolved.alias.as_str())?
        .clone();
    if !resolved.effort.is_empty() {
        row["thinking"] = json!(resolved.effort);
    }
    Some(row)
}

/// Capture the stored alias before the resume/profile runtime replaces the store snapshot.
pub(crate) fn recorded(session: &Value) -> Option<Choice> {
    Some(Choice {
        alias: session["model"].as_str()?.into(),
        effort: session["reasoningEffort"].as_str().unwrap_or("off").into(),
        display_name: String::new(),
        name: String::new(),
        provider: String::new(),
        provider_key: String::new(),
    })
}

fn actual(result: &Value) -> Option<Choice> {
    let model = result.pointer("/runtime/config/activeModel")?;
    Some(Choice {
        alias: model["alias"].as_str()?.into(),
        effort: model["thinking"].as_str()?.into(),
        display_name: model["displayName"].as_str().unwrap_or_default().into(),
        name: model["name"].as_str().unwrap_or_default().into(),
        provider: String::new(),
        provider_key: String::new(),
    })
}

/// Resolve the backend from 3.1.4's session-specific config fields. Its public `ModelConfigView`
/// deliberately omits provider; `models` and `providers` in this separate reply retain the
/// resolved config. Require the row's alias and name to match the runtime view across the reads.
fn provenance(
    fields: &Value,
    choice: &Choice,
    mistral_proxy_route: Option<&str>,
) -> Option<(String, String)> {
    let rows = fields["fields"].as_array()?;
    let value = |name| {
        rows.iter()
            .find(|row| row["name"] == name)
            .map(|row| &row["value"])
    };
    let models = value("models")?;
    let model = if let Some(map) = models.as_object() {
        map.get(&choice.alias)?
    } else {
        let rows = models.as_array()?;
        let mut matches = rows.iter().filter(|row| row["alias"] == choice.alias);
        let model = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        model
    };
    if model["alias"] != choice.alias || model["name"] != choice.name {
        return None;
    }
    let provider = model["provider"].as_str().filter(|name| !name.is_empty())?;
    let mut providers = value("providers")?
        .as_array()?
        .iter()
        .filter(|row| row["name"] == provider);
    let route = providers.next()?;
    if providers.next().is_some() || route["api_base"].as_str().is_none_or(str::is_empty) {
        return None;
    }
    // These fields determine where and how Vibe routes the model. A digest avoids writing
    // endpoint URLs, environment variable names or custom headers into chat choice records.
    let api_base = if provider == "mistral" && mistral_proxy_route == route["api_base"].as_str() {
        json!("vibe-work:mistral-proxy")
    } else {
        route["api_base"].clone()
    };
    let route = json!({
        "provider":provider,
        "api_base":api_base,
        "api_style":route["api_style"],
        "backend":route["backend"],
        "api_key_env_var":route["api_key_env_var"],
        "project_id":route["project_id"],
        "region":route["region"],
        "extra_headers":route["extra_headers"],
    });
    let bytes = serde_json::to_vec(&route).ok()?;
    Some((provider.into(), blake3::hash(&bytes).to_hex().to_string()))
}

async fn identified(shared: &Shared, id: &str, mut choice: Choice) -> Choice {
    if let Ok(reply) = shared
        .gate
        .call("config/fields/read", json!({"sessionId":id}), None)
        .await
        && let Some((provider, key)) = provenance(
            &reply.result,
            &choice,
            shared.model_choices.mistral_proxy_route.as_deref(),
        )
    {
        choice.provider = provider;
        choice.provider_key = key;
    }
    choice
}

async fn identified_actual(shared: &Shared, id: &str, result: &Value) -> Option<Choice> {
    Some(identified(shared, id, actual(result)?).await)
}

fn accepted(result: &Value) -> Result<(), CoreError> {
    if let Some(failures) = result["failures"].as_array().filter(|v| !v.is_empty()) {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            failures
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    if result["rejected"] == true {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "The model choice was refused by this configuration.",
        ));
    }
    Ok(())
}

/// Replays only a pending pick, through the same closed gate, after restoring the agent profile.
/// A pin in a newly managed/profile layer must refuse visibly, never run a different choice silently.
async fn apply(shared: &Shared, id: &str, choice: &Choice) -> Result<Value, CoreError> {
    let read = "runtime/read";
    shared
        .gate
        .call(read, json!({"sessionId":id}), None)
        .await
        .map_err(|e| CoreError::from_gate(read, &e))?;
    let method = "config/model/write";
    let reply = shared
        .gate
        .call(
            method,
            json!({"sessionId":id,"sessionOnly":true,
        "modelAlias":choice.alias,"reasoningEffort":choice.effort}),
            None,
        )
        .await
        .map_err(|e| CoreError::from_gate(method, &e))?;
    accepted(&reply.result)?;
    if !identified_actual(shared, id, &reply.result)
        .await
        .is_some_and(|actual| choice.matches(&actual))
    {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "This configuration did not restore the chat's model or thinking choice.",
        ));
    }
    Ok(reply.result["runtime"].clone())
}

/// Deferred worktree setup can answer turn/start before pinning. Retire only a confirmed durable choice.
pub(crate) async fn turn_started(shared: &Arc<Shared>, params: &Value) {
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    // Remember the served identity and retire confirmed pending/fallback pins in one commit.
    let served = if let Some(runtime) = shared.store.runtime(id) {
        identified_actual(shared, id, &json!({"runtime":runtime})).await
    } else {
        None
    };
    if let Some(choice) = &served
        && moved_off(shared, id, choice).await
    {
        return;
    }
    let expected = shared.model_choices.records.get(id).replay();
    let pinned = if let Some(choice) = &expected {
        shared
            .gate
            .call(
                "session/read",
                json!({"sessionId":id,"history":{"limit":1},"turns":{"limit":1}}),
                None,
            )
            .await
            .is_ok_and(|reply| {
                let session = &reply.result["state"]["session"];
                session["model"].as_str() == Some(choice.alias.as_str())
                    && session["reasoningEffort"].as_str() == Some(choice.effort.as_str())
            })
    } else {
        false
    };
    if let Err(error) = shared.model_choices.records.update(id, |record| {
        // A newer pick made during session/read must not be retired by the older turn.
        if record.replay() != expected {
            return;
        }
        if let Some(choice) = served.filter(|choice| !choice.provider_key.is_empty()) {
            record.remembered = Some(choice);
        }
        if pinned {
            record.pending = None;
            record.fallback = None;
        }
    }) {
        tracing::warn!(%error, "chat model identity could not be saved");
    }
}

/// Vibe resolves the model again when a turn starts. A model that left Vibe's list after the send check (its runtime
/// view can trail its own model list by a few seconds) runs the turn on Vibe's own fallback. Then the chat gets the
/// same notice and Settings default as at reopen, and `true` is returned. `choice` is the model the check let through.
async fn moved_off(shared: &Arc<Shared>, id: &str, choice: &Choice) -> bool {
    let Ok(runtime) = restore::read(shared, id).await else {
        return false;
    };
    // Without a model list (an older runtime shape) nothing shows that the model left it.
    if !runtime
        .pointer("/config/models")
        .is_some_and(Value::is_array)
    {
        return false;
    }
    let current = identified_actual(shared, id, &json!({"runtime":runtime})).await;
    if restore::offered(&runtime, choice)
        && current.is_some_and(|current| choice.same_model(&current))
    {
        return false;
    }
    let choice = shared.model_choices.named(choice.clone());
    let runtime = restore::fallback(shared, id, &choice, runtime).await;
    publish(shared, id, runtime).await;
    true
}
