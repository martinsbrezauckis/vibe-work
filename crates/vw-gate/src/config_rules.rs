//! Typed handlers for `config/write` and `config/model/write` (class P1).
//!
//! Depends on: shape, profile, error, canonical. Called by: privileged.
//! Invariants:
//! - `config/write` accepts only paths on [`CONFIG_KEYS`], each with a value of its own type, and
//!   `/models/<configured alias>/top_p`, `/max_output_tokens` and `/auto_compact_threshold` with a value in
//!   range.
//!   Anything that can run code or redirect traffic (`/mcp_servers`, `/hooks`, `/providers`,
//!   `/models`, `/tools/...`, proxy and TLS settings) is not listed, so it is refused.
//! - One bad op refuses the whole call; nothing is partly applied.
//! - `config/model/write` accepts only a model alias Vibe reported in `runtime/read` and one of
//!   the five reasoning levels, with `sessionOnly: true`; chat picks never write the user default.
//! - Making `auto-approve` the default profile is an escalation (needs a native confirmation).
//! - The two privacy switches (`/enable_telemetry`, `/experiments/enable`) take a
//!   boolean. Turning one OFF is free (less data leaves the machine). Turning one ON, and removing
//!   the key (which falls back to Vibe's default, ON), is an escalation: it needs a native
//!   confirmation, so neither the Debug API nor a page can switch data sharing on silently.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::error::{DenyReason, GateError};
use crate::profile::{escalates, is_profile};
use crate::shape::{Field, Kind, check, invalid, opt, req};

/// What value a config path takes.
#[derive(Clone, Copy)]
enum KeyType {
    Bool,
    Theme,
    DefaultAgent,
    ActiveModel,
    /// A data-sharing switch: off is free, on (or removing the key) is an escalation.
    Privacy,
    /// Seconds, 0 to 3600 (`api_stream_idle_timeout`, 0 = off).
    Seconds,
}

/// The config keys the app may write: display and behaviour settings plus the model pick.
/// Source of the names: `ConfigView` (generated protocol type). Not listed on purpose:
/// `enable_system_trust_store` (TLS trust), `log_level`, every `experimental_*`, and all
/// structured keys. The privacy switches are listed as [`KeyType::Privacy`].
const CONFIG_KEYS: &[(&str, KeyType)] = &[
    ("/theme", KeyType::Theme),
    ("/disable_welcome_banner_animation", KeyType::Bool),
    ("/show_greeting", KeyType::Bool),
    ("/autocopy_to_clipboard", KeyType::Bool),
    ("/file_watcher_for_autocomplete", KeyType::Bool),
    ("/ask_confirmation_on_exit", KeyType::Bool),
    ("/voice_mode_enabled", KeyType::Bool),
    ("/narrator_enabled", KeyType::Bool),
    ("/show_thinking_nodes", KeyType::Bool),
    ("/show_subagent_status_list", KeyType::Bool),
    ("/enable_update_checks", KeyType::Bool),
    ("/enable_notifications", KeyType::Bool),
    ("/default_agent", KeyType::DefaultAgent),
    ("/active_model", KeyType::ActiveModel),
    // Vibe 2.26.1: what the agent may use, and when a silent answer counts as stalled.
    ("/enable_background_processes", KeyType::Bool),
    ("/enable_subagents", KeyType::Bool),
    ("/api_stream_idle_timeout", KeyType::Seconds),
    ("/enable_telemetry", KeyType::Privacy),
    ("/experiments/enable", KeyType::Privacy),
];

/// What the confirmation dialog says when a data-sharing switch is turned on.
const PRIVACY_ON: &str = "Turn on data sharing with Mistral: usage telemetry and the experiments that decide which features you get";

/// Most ops in one `config/write`.
const MAX_OPS: usize = 16;

/// Reasoning levels Vibe accepts (`ModelConfigView.thinkingLevels`).
const REASONING_LEVELS: &[&str] = &["off", "low", "medium", "high", "xhigh", "max"];

const CONFIG_WRITE_FIELDS: &[Field] = &[
    req("sessionId", Kind::Text(128)),
    req("ops", Kind::Any),
    opt("reason", Kind::Text(200)),
    opt("reloadRuntime", Kind::Bool),
];

const MODEL_WRITE_FIELDS: &[Field] = &[
    req("sessionId", Kind::Text(128)),
    req("sessionOnly", Kind::Bool),
    opt("modelAlias", Kind::Text(128)),
    opt("reasoningEffort", Kind::OneOf(REASONING_LEVELS)),
];

fn policy(method: &str, rule: &str) -> GateError {
    GateError::Denied {
        method: method.to_owned(),
        reason: DenyReason::Policy(rule.to_owned()),
    }
}

/// Validates `config/write`. Returns a confirmation summary when the call escalates.
pub(crate) fn config_write(
    method: &str,
    params: &Value,
    aliases: &HashSet<String>,
) -> Result<Option<String>, GateError> {
    let object = check(method, params, CONFIG_WRITE_FIELDS)?;
    let ops = object
        .get("ops")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(method, "`ops` must be an array"))?;
    if ops.is_empty() || ops.len() > MAX_OPS {
        return Err(invalid(
            method,
            format!("`ops` needs 1 to {MAX_OPS} entries"),
        ));
    }
    let mut escalation = None;
    for op in ops {
        if let Some(summary) = check_op(method, op, aliases)? {
            escalation = Some(summary);
        }
    }
    Ok(escalation)
}

fn check_op(
    method: &str,
    op: &Value,
    aliases: &HashSet<String>,
) -> Result<Option<String>, GateError> {
    let map: &Map<String, Value> = op
        .as_object()
        .ok_or_else(|| invalid(method, "each op must be an object"))?;
    if let Some(extra) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "op" | "path" | "value" | "targetLayer"))
    {
        return Err(invalid(method, format!("unknown op field `{extra}`")));
    }
    // Only the default layer is verified; any other layer name is unexplored.
    if map.get("targetLayer").is_some_and(|v| !v.is_null()) {
        return Err(policy(method, "targetLayer is not allowed"));
    }
    let kind = map.get("op").and_then(Value::as_str);
    let path = map
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(method, "each op needs a `path` string"))?;
    if let Some(field) = model_field(path, aliases) {
        return check_model_field(method, kind, field, map.get("value"));
    }
    let Some((_, value_kind)) = CONFIG_KEYS.iter().find(|(p, _)| *p == path) else {
        return Err(policy(method, "config path is not on the allowlist"));
    };
    match kind {
        Some("remove") => {
            if map.get("value").is_some() {
                return Err(invalid(method, "`remove` takes no value"));
            }
            // Removing a privacy key falls back to Vibe's default, which is ON.
            Ok(matches!(value_kind, KeyType::Privacy).then(|| PRIVACY_ON.to_owned()))
        }
        Some("set") => {
            let value = map
                .get("value")
                .ok_or_else(|| invalid(method, "`set` needs a value"))?;
            check_value(method, *value_kind, value, aliases)
        }
        _ => Err(invalid(method, "`op` must be `set` or `remove`")),
    }
}

fn check_value(
    method: &str,
    kind: KeyType,
    value: &Value,
    aliases: &HashSet<String>,
) -> Result<Option<String>, GateError> {
    let bad = || invalid(method, "value has the wrong type for this config key");
    match kind {
        KeyType::Bool => value.is_boolean().then_some(None).ok_or_else(bad),
        KeyType::Privacy => match value.as_bool() {
            Some(on) => Ok(on.then(|| PRIVACY_ON.to_owned())),
            None => Err(bad()),
        },
        KeyType::Theme => {
            let ok = value.as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            });
            ok.then_some(None).ok_or_else(bad)
        }
        KeyType::DefaultAgent => match value.as_str() {
            Some(name) if is_profile(name) => Ok(escalates(name).then(|| {
                format!(
                    "Make {} the default access mode: new sessions would run every tool without asking",
                    vw_vocabulary::label("accessModes", "auto-approve")
                )
            })),
            _ => Err(bad()),
        },
        KeyType::Seconds => value
            .as_f64()
            .filter(|s| (0.0..=3600.0).contains(s))
            .map(|_| None)
            .ok_or_else(bad),
        KeyType::ActiveModel => match value.as_str() {
            Some(alias) if aliases.contains(alias) => Ok(None),
            _ => Err(bad()),
        },
    }
}

/// Validates `config/model/write`.
pub(crate) fn model_write(
    method: &str,
    params: &Value,
    aliases: &HashSet<String>,
) -> Result<(), GateError> {
    let object = check(method, params, MODEL_WRITE_FIELDS)?;
    if object.get("sessionOnly") != Some(&Value::Bool(true)) {
        return Err(policy(method, "model picks require sessionOnly: true"));
    }
    let alias = object.get("modelAlias").filter(|v| !v.is_null());
    let effort = object.get("reasoningEffort").filter(|v| !v.is_null());
    if alias.is_none() && effort.is_none() {
        return Err(invalid(method, "needs `modelAlias` or `reasoningEffort`"));
    }
    if let Some(alias) = alias.and_then(Value::as_str)
        && !aliases.contains(alias)
    {
        return Err(policy(
            method,
            "model alias is not one of the configured models",
        ));
    }
    Ok(())
}

/// The per-model fields the app may write (Vibe `ModelConfig`): nucleus sampling, the output cap and the compaction
/// point.
#[derive(Clone, Copy)]
enum ModelField {
    /// `top_p`: (0, 1].
    TopP,
    /// `max_output_tokens`: 1 to 1,048,576.
    MaxOutputTokens,
    /// `auto_compact_threshold` (Vibe 3.0.3): a whole token count in [`COMPACT_THRESHOLD`]. Vibe refuses 0 and a
    /// count outside the range its model's window allows; the window is Vibe's to know, so this rule only keeps the
    /// value a plain token count and the window check stays Vibe's.
    AutoCompactThreshold,
}

/// The compaction point's outer range: Vibe's smallest allowed threshold for any window
/// (`completion_profile/bounds.py` `THRESHOLD_MIN_FLOOR`) up to four times the largest window Mistral serves.
const COMPACT_THRESHOLD: std::ops::RangeInclusive<u64> = 10_000..=4_194_304;

/// `/models/<alias>/<field>` for a configured alias (the JSON-pointer token unescaped) and an allowed field.
fn model_field(path: &str, aliases: &HashSet<String>) -> Option<ModelField> {
    let rest = path.strip_prefix("/models/")?;
    let (token, field) = rest.rsplit_once('/')?;
    if token.contains('/') {
        return None;
    }
    let alias = token.replace("~1", "/").replace("~0", "~");
    if !aliases.contains(&alias) {
        return None;
    }
    match field {
        "top_p" => Some(ModelField::TopP),
        "max_output_tokens" => Some(ModelField::MaxOutputTokens),
        "auto_compact_threshold" => Some(ModelField::AutoCompactThreshold),
        _ => None,
    }
}

/// `set` with a value in range, or `remove` (back to the provider default). Never an escalation.
fn check_model_field(
    method: &str,
    op: Option<&str>,
    field: ModelField,
    value: Option<&Value>,
) -> Result<Option<String>, GateError> {
    let bad = || {
        invalid(
            method,
            "value has the wrong type or range for this model field",
        )
    };
    match (op, value) {
        (Some("remove"), None) => Ok(None),
        (Some("remove"), Some(_)) => Err(invalid(method, "`remove` takes no value")),
        (Some("set"), Some(v)) => {
            let ok = match field {
                ModelField::TopP => v.as_f64().is_some_and(|p| p > 0.0 && p <= 1.0),
                ModelField::MaxOutputTokens => {
                    v.as_u64().is_some_and(|n| (1..=1_048_576).contains(&n))
                }
                ModelField::AutoCompactThreshold => {
                    v.as_u64().is_some_and(|n| COMPACT_THRESHOLD.contains(&n))
                }
            };
            if ok { Ok(None) } else { Err(bad()) }
        }
        (Some("set"), None) => Err(invalid(method, "`set` needs a value")),
        _ => Err(invalid(method, "`op` must be `set` or `remove`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_picks_require_session_scope_and_keep_the_closed_shape() {
        let aliases = HashSet::from(["large-4".to_owned()]);
        let method = "config/model/write";
        for choice in [
            json!({"modelAlias":"large-4"}),
            json!({"reasoningEffort":"high"}),
        ] {
            let mut params = choice;
            params["sessionId"] = json!("chat-a");
            assert!(
                model_write(method, &params, &aliases).is_err(),
                "missing scope"
            );
            for scope in [json!(false), json!(null), json!("true"), json!(1)] {
                params["sessionOnly"] = scope;
                assert!(model_write(method, &params, &aliases).is_err());
            }
            params["sessionOnly"] = json!(true);
            assert!(model_write(method, &params, &aliases).is_ok());
            params["targetLayer"] = json!("user");
            assert!(model_write(method, &params, &aliases).is_err());
        }
        for params in [
            json!({"sessionId":"a","sessionOnly":true}),
            json!({"sessionId":"a","sessionOnly":true,"modelAlias":"unknown"}),
            json!({"sessionId":"a","sessionOnly":true,"reasoningEffort":"extreme"}),
        ] {
            assert!(model_write(method, &params, &aliases).is_err());
        }
    }
}
