//! Which model the core's own direct Mistral calls use when Vibe Work names none: the chat's active model.
//!
//! The commit message draft has no model of its own: it uses the model the chat runs (Vibe's
//! `runtime.config.activeModel.name`, from Mistral's model list), unless the core config names one. Library searches
//! and the image tool keep Mistral's own moving `-latest` aliases (Mistral decides what they point to), and when
//! Mistral refuses that model they try once more with the chat's active model. No model name chosen by Vibe Work
//! can break when Mistral renames or retires a model.
//!
//! Depends on: the session store's cached runtimes and the gated `runtime/read`. Called by: `git_draft`, `work`,
//! `agent_tools::work_tools`.

use serde_json::{Value, json};

use crate::core::Shared;

/// The provider model name (`name`) of the runtime's active model.
pub(crate) fn active_in(runtime: &Value) -> Option<String> {
    let name = runtime
        .pointer("/config/activeModel/name")?
        .as_str()?
        .trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The provider model name of the runtime's Settings default (`defaultModelAlias` found in its model list).
pub(crate) fn default_in(runtime: &Value) -> Option<String> {
    let alias = runtime.pointer("/config/defaultModelAlias")?.as_str()?;
    runtime
        .pointer("/config/models")?
        .as_array()?
        .iter()
        .find(|model| model["alias"].as_str() == Some(alias))
        .and_then(|model| model["name"].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The chat's active model: from `session`'s cached runtime, else its gated `runtime/read`. Without a chat (the
/// Libraries window), the Settings default of any open chat's runtime. `None` when no runtime is known.
pub(crate) async fn active_model(shared: &Shared, session: Option<&str>) -> Option<String> {
    if let Some(id) = session {
        if let Some(name) = shared.store.runtime(id).as_ref().and_then(active_in) {
            return Some(name);
        }
        let reply = shared
            .gate
            .call("runtime/read", json!({ "sessionId": id }), None)
            .await
            .ok()?;
        return reply.result.get("runtime").and_then(active_in);
    }
    shared
        .store
        .statuses()
        .into_iter()
        .filter_map(|(id, _)| shared.store.runtime(&id))
        .find_map(|runtime| default_in(&runtime).or_else(|| active_in(&runtime)))
}

/// Whether Mistral refused a work request in a way a different model can fix: 400/422 (invalid model) or 404
/// (unknown model).
pub(crate) fn model_refused(error: &vw_keys::WorkFailure) -> bool {
    use vw_mistral::work::WorkError;
    matches!(
        error,
        vw_keys::WorkFailure::Work(WorkError::Rejected(400 | 422) | WorkError::NotFound)
    )
}

/// The model to try after `tried` was refused: the chat's active model when it is a different one.
pub(crate) async fn retry_model(
    shared: &Shared,
    session: Option<&str>,
    tried: &str,
) -> Option<String> {
    active_model(shared, session)
        .await
        .filter(|active| active != tried)
}

#[cfg(test)]
#[path = "utility_models_tests.rs"]
mod tests;
