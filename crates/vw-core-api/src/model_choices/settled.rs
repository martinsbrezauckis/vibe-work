//! Core's model-restore outcome on runtime snapshots, consumed by post-unlock notice judgement.
//! Uses the saved-choice restore and per-chat dispatch locks; a detached or busy read never applies a choice.
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn key_usable(shared: &Shared) -> bool {
    shared
        .config
        .keys
        .as_ref()
        .is_none_or(|keys| keys.speech_available())
}

/// Raw Vibe events during restoration cannot establish the saved choice's outcome.
pub(crate) fn pending_runtime(shared: &Shared, id: &str, runtime: &mut Value) {
    if runtime.is_object() && (!key_usable(shared) || shared.channels.is_restoring(id)) {
        runtime["modelChoiceRestore"] = json!("pending");
    }
}

/// A checked publication follows the actual saved-choice application or its visible fallback.
pub(crate) fn certify(shared: &Shared, id: &str, runtime: &mut Value) {
    if !runtime.is_object() {
        return;
    }
    let waiting = shared
        .model_choices
        .records
        .get(id)
        .expected()
        .is_some_and(|choice| shared.model_choices.cold_replay_matches(id, &choice));
    runtime["modelChoiceRestore"] = json!(if waiting
        || !key_usable(shared)
        || runtime["modelChoiceRestore"] == "pending"
    {
        "pending"
    } else {
        "checked"
    });
}

/// Called under the chat's send and restore locks before publishing a runtime/read or reload reply.
/// The metadata is Core-owned; Vibe's runtime alone cannot certify that a saved pick was restored.
pub(crate) async fn finish_read(shared: &Arc<Shared>, id: &str, result: &mut Value) {
    let Some(mut runtime) = result.get("runtime").filter(|v| v.is_object()).cloned() else {
        return;
    };
    if shared.channels.has_channel(id)
        && key_usable(shared)
        && !crate::recovery::working(shared, id)
    {
        if shared.model_choices.records.get(id).expected().is_some() {
            runtime = super::restore::restore(shared, id, runtime, None).await;
        }
        certify(shared, id, &mut runtime);
    } else {
        runtime["modelChoiceRestore"] = json!("pending");
    }
    result["modelChoiceRestore"] = runtime["modelChoiceRestore"].clone();
    result["runtime"] = runtime;
}
