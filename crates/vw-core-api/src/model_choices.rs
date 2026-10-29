//! Pending chat model/thinking choices, until Vibe pins them at the next turn start.
//! Depends on the gated RPC, private atomic files and `sessions_dir`; called by sessions and restore.
//! Vibe 3.0.2's sessionOnly writes only memory. Replay before publishing runtime; failed model
//! restores alert without closing the chat. No user config or Vibe session files are edited here.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

mod handoff;
pub(crate) use handoff::{forked, handed_off};
mod restore;
pub(crate) use restore::restore;
mod live;
pub(crate) use live::{admission, before, pick, publish, selected};

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
        if self.effort == "off" {
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

/// Choices not yet made durable by Vibe. The desktop always supplies `sessions_dir`.
struct Records {
    pending: Mutex<BTreeMap<String, Choice>>,
    file: Option<PathBuf>,
}

impl Records {
    fn load(dir: Option<&std::path::Path>, name: &str) -> Self {
        let file = dir.map(|dir| dir.join(name));
        let pending = file
            .as_ref()
            .and_then(|file| match load(file) {
                Ok(pending) => Some(pending),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    tracing::warn!(%error, "saved chat model choices ignored");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            pending: Mutex::new(pending),
            file,
        }
    }

    fn change(&self, id: &str, choice: Option<Choice>) -> Result<(), CoreError> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.get(id) == choice.as_ref() {
            return Ok(());
        }
        let mut updated = pending.clone();
        if let Some(choice) = choice {
            updated.insert(id.into(), choice);
        } else {
            updated.remove(id);
        }
        if let Some(file) = &self.file {
            let write = || -> Result<(), Box<dyn std::error::Error>> {
                if let Some(dir) = file.parent() {
                    vw_platform::ensure_private_dir(dir)?;
                }
                vw_platform::replace_private(file, &serde_json::to_vec(&updated)?)?;
                Ok(())
            };
            write().map_err(|_| {
                CoreError::new(
                    ErrorCode::Internal,
                    "The chat's model choice could not be saved. Try picking it again.",
                )
            })?;
        }
        *pending = updated;
        Ok(())
    }

    pub(crate) fn forget(&self, id: &str) {
        if let Err(error) = self.change(id, None) {
            tracing::warn!(%error, "pending model record not removed");
        }
    }

    fn get(&self, id: &str) -> Option<Choice> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }
}

/// Accepted chat picks and recovery defaults awaiting Vibe's durable turn pins.
pub(crate) struct Choices {
    pending: Records,
    // Keep the served model and display name after Vibe pins the alias. A hosted alias can be retargeted.
    remembered: Records,
    // Kept separately so a refused user pick is removed, even before the next turn.
    // Without this, a second reopen could restore an older, still-valid Vibe pin.
    fallbacks: Records,
    handed_off: Mutex<std::collections::BTreeSet<(String, String)>>,
}

impl Choices {
    pub(crate) fn load(dir: Option<&std::path::Path>) -> Self {
        Self {
            pending: Records::load(dir, "pending-models.json"),
            remembered: Records::load(dir, "chat-models.json"),
            fallbacks: Records::load(dir, "model-fallbacks.json"),
            handed_off: Mutex::default(),
        }
    }

    pub(crate) fn forget(&self, id: &str) {
        self.pending.forget(id);
        self.remembered.forget(id);
        self.fallbacks.forget(id);
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
        self.pending.change(id, Some(choice.clone()))?;
        self.remembered.change(id, Some(choice))?;
        self.fallbacks.change(id, None)
    }
}

fn load(file: &std::path::Path) -> std::io::Result<BTreeMap<String, Choice>> {
    const MAX: u64 = 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(file)?
        .take(MAX + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX {
        return Err(std::io::Error::other("pending-models.json exceeds 1 MiB"));
    }
    let pending: BTreeMap<String, Choice> = serde_json::from_slice(&bytes)?;
    if pending.iter().any(|(id, choice)| {
        id.is_empty()
            || id.len() > 128
            || choice.alias.is_empty()
            || choice.alias.len() > 128
            || !matches!(
                choice.effort.as_str(),
                "off" | "low" | "medium" | "high" | "xhigh" | "max"
            )
    }) {
        return Err(std::io::Error::other("invalid saved chat model choice"));
    }
    Ok(pending)
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
    // Keep the provider identity as well as Vibe's alias, which can be retargeted later.
    if let Some(runtime) = shared.store.runtime(id)
        && let Some(choice) = actual(&json!({"runtime":runtime}))
        && let Err(error) = shared.model_choices.remembered.change(id, Some(choice))
    {
        tracing::warn!(%error, "chat model identity could not be saved");
    }
    let choice = shared
        .model_choices
        .fallbacks
        .get(id)
        .or_else(|| shared.model_choices.pending.get(id));
    let Some(choice) = choice else {
        return;
    };
    if let Ok(reply) = shared
        .gate
        .call(
            "session/read",
            json!({"sessionId":id,"history":{"limit":1},"turns":{"limit":1}}),
            None,
        )
        .await
    {
        let session = &reply.result["state"]["session"];
        if session["model"].as_str() == Some(choice.alias.as_str())
            && session["reasoningEffort"].as_str() == Some(choice.effort.as_str())
        {
            shared.model_choices.pending.forget(id);
            shared.model_choices.fallbacks.forget(id);
        }
    }
}
