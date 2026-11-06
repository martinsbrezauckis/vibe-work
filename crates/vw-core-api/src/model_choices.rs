//! Saved chat model/thinking choices in one record, replayed until Vibe pins the next turn.
//! Depends on the gated RPC, private atomic files and `sessions_dir`; called by sessions and restore.
//! Vibe 3.0.2's sessionOnly writes only memory. Replay before publishing runtime; failed model
//! restores alert without closing the chat. No user config or Vibe session files are edited here.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

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

mod live;
pub(crate) use live::{admission, before, pick, publish, selected};

#[cfg(test)]
mod persistence_tests;

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
}

impl Choice {
    fn matches(&self, other: &Self) -> bool {
        self.same_model(other) && self.effort == other.effort
    }

    fn same_model(&self, other: &Self) -> bool {
        self.alias == other.alias && (self.name.is_empty() || self.name == other.name)
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
    handed_off: Mutex<std::collections::BTreeSet<(String, String)>>,
    /// Display names by alias from the model lists seen in this run ([`Self::learn`]).
    names: Mutex<BTreeMap<String, String>>,
}

/// Aliases whose display name is remembered (a model list holds a few dozen at most).
const MAX_NAMES: usize = 512;

impl Choices {
    pub(crate) fn load(dir: Option<&std::path::Path>) -> Self {
        Self {
            records: Records::load(dir),
            handed_off: Mutex::default(),
            names: Mutex::default(),
        }
    }

    /// Remembers the display name of every model in a runtime's or configuration's model list (`config.models`), so
    /// a model that later leaves Vibe's list can still be named ([`Self::named`]).
    pub(crate) fn learn(&self, runtime: &Value) {
        let Some(models) = runtime.pointer("/config/models").and_then(Value::as_array) else {
            return;
        };
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
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
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&choice.alias)
            .cloned()
            .or_else(|| self.records.named(&choice.alias));
        if let Some(name) = known {
            choice.display_name = name;
        }
        choice
    }

    pub(crate) fn forget(&self, id: &str) {
        self.records.forget(id);
        self.handed_off
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(old, new)| old != id && new != id);
    }

    /// Records accepted picks only; turns retire them after Vibe confirms its durable pins.
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

/// Capture the stored alias before the resume/profile runtime replaces the store snapshot.
pub(crate) fn recorded(session: &Value) -> Option<Choice> {
    Some(Choice {
        alias: session["model"].as_str()?.into(),
        effort: session["reasoningEffort"].as_str().unwrap_or("off").into(),
        display_name: String::new(),
        name: String::new(),
    })
}

fn actual(result: &Value) -> Option<Choice> {
    let model = result.pointer("/runtime/config/activeModel")?;
    Some(Choice {
        alias: model["alias"].as_str()?.into(),
        effort: model["thinking"].as_str()?.into(),
        display_name: model["displayName"].as_str().unwrap_or_default().into(),
        name: model["name"].as_str().unwrap_or_default().into(),
    })
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
    if !actual(&reply.result).is_some_and(|actual| choice.matches(&actual)) {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "This configuration did not restore the chat's model or thinking choice.",
        ));
    }
    Ok(reply.result["runtime"].clone())
}

/// Deferred worktree setup can answer turn/start before pinning. Retire only a confirmed durable choice.
pub(crate) async fn turn_started(shared: &Shared, params: &Value) {
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    // Remember the served identity and retire confirmed pending/fallback pins in one commit.
    let served = shared
        .store
        .runtime(id)
        .and_then(|runtime| actual(&json!({"runtime":runtime})));
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
        if let Some(choice) = served {
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
