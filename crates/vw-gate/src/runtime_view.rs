//! Reads the configured model aliases out of a runtime snapshot.
//!
//! Depends on: `serde_json`. Called by: gate (after `runtime/read`, `config/write`,
//! `config/model/write`, and `Gate::observe_runtime`).
//! Source: `RuntimeSnapshot.config` (`ConfigView.models[].alias`, `activeModel.alias`).

use std::collections::HashSet;

use serde_json::Value;

/// Aliases found in `value`, which is either a runtime snapshot or an object holding one under
/// `runtime` (the shape of `runtime/read`, `config/write` and `runtime/updated` results).
/// `None` when the value carries no model list (the known set must then stay as it was).
pub(crate) fn model_aliases(value: &Value) -> Option<HashSet<String>> {
    let config = value.get("runtime").unwrap_or(value).get("config")?;
    let models = config.get("models")?.as_array()?;
    let mut aliases: HashSet<String> = models
        .iter()
        .filter_map(|m| m.get("alias").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    if let Some(active) = config
        .get("activeModel")
        .and_then(|m| m.get("alias"))
        .and_then(Value::as_str)
    {
        aliases.insert(active.to_owned());
    }
    Some(aliases)
}
