//! Applies the Settings default when a picked model differs from what Vibe will run.
//! Called by dispatch on Send and model selection; runtime checks never block chat work.
use super::{Choice, actual};
use crate::core::Shared;
use crate::error::CoreError;
use serde_json::{Value, json};
use std::sync::Arc;
use vw_gate::Reply;

pub(crate) fn admission(method: &str, params: &Value) -> Value {
    let mut checked = params.clone();
    if method == "config/model/write" {
        if let Some(fields) = checked.as_object_mut() {
            fields.remove("modelAlias");
        }
        checked["reasoningEffort"] = params
            .get("reasoningEffort")
            .cloned()
            .unwrap_or(json!("off"));
    }
    checked
}

pub(crate) async fn publish(shared: &Arc<Shared>, id: &str, runtime: Value) {
    let previous_mode = crate::chrome_control::chat_mode(shared, id);
    shared.access.observe_runtime(id, &runtime);
    crate::chrome_control::mode_changed(shared, id, previous_mode.as_deref());
    shared.model_choices.learn(&runtime);
    let applied = shared.store.apply(vw_session::Notification::new(
        "runtime/updated",
        json!({"sessionId":id,"runtime":runtime}),
    ));
    let _ = shared.loads.send(applied).await;
}

pub(crate) async fn before(
    shared: &Arc<Shared>,
    method: &str,
    params: &Value,
) -> Result<(), CoreError> {
    if !matches!(
        method,
        "turn/start" | "session/turn/enqueue" | "session/turn/queue/resume"
    ) {
        return Ok(());
    }
    let Some(id) = params["sessionId"].as_str() else {
        return Ok(());
    };
    let choices = &shared.model_choices;
    let prior = choices.records.get(id);
    let runtime = match super::restore::read(shared, id).await {
        Ok(runtime) => runtime,
        Err(error) => {
            if prior
                .expected()
                .is_some_and(|choice| !choice.provider_key.is_empty())
            {
                return Err(error);
            }
            tracing::warn!(%error, "chat model could not be read before sending");
            return Ok(());
        }
    };
    super::restore::mark_cold_replay(shared, id, &runtime, prior.pending.as_ref());
    let (runtime, cold_missing) = if let Some(choice) = prior.expected() {
        super::restore::after_ready(shared, id, runtime, &choice).await?
    } else {
        (runtime, false)
    };
    super::retired::migrate(&choices.records, id, &runtime);
    let record = choices.records.get(id);
    let expected = record.expected();
    let expected = expected.or_else(|| super::restore::unavailable_default(&runtime["config"]));
    let current = super::identified_actual(shared, id, &json!({"runtime":runtime})).await;
    let runtime = if let Some(choice) =
        expected.filter(|choice| super::restore::differs(&runtime, choice, current.as_ref()))
    {
        if record.pending.as_ref() == Some(&choice)
            && !choice.provider_key.is_empty()
            && super::restore::replayable(shared, id, &runtime, &choice).await
        {
            match super::apply(shared, id, &choice).await {
                Ok(applied) => applied,
                Err(error) => {
                    if !(cold_missing || choices.cold_replay_matches(id, &choice))
                        || super::restore::backend_changed(shared, id, &choice).await
                    {
                        super::restore::fallback(shared, id, &choice, runtime).await
                    } else {
                        return Err(error);
                    }
                }
            }
        } else {
            super::restore::fallback(shared, id, &choice, runtime).await
        }
    } else {
        runtime
    };
    choices.clear_cold_replay(id);
    publish(shared, id, runtime).await;
    Ok(())
}

/// A stale picker or a changed provider identity uses the same fallback as reopen.
pub(crate) async fn pick(
    shared: &Arc<Shared>,
    params: &Value,
    selected: Option<Choice>,
) -> Result<Reply, CoreError> {
    let method = "config/model/write";
    shared
        .gate
        .check(method, &admission(method, params), None)
        .map_err(|e| CoreError::from_gate(method, &e))?;
    let id = params["sessionId"]
        .as_str()
        .ok_or_else(|| CoreError::bad_result(method, "sessionId"))?;
    let runtime = super::restore::read(shared, id).await?;
    let selected = selected.or_else(|| selection(params, &runtime));
    let selected = if let Some(choice) = selected {
        Some(super::identified(shared, id, choice).await)
    } else {
        None
    };
    if let Some(choice) = selected
        .as_ref()
        .filter(|choice| !super::restore::offered(&runtime, choice))
    {
        return Ok(replacement(shared, id, choice, runtime).await);
    }
    match shared.gate.call(method, params.clone(), None).await {
        Ok(reply) => {
            let confirmed = super::identified_actual(shared, id, &reply.result).await;
            if let Some(choice) = selected.as_ref()
                && confirmed
                    .as_ref()
                    .is_none_or(|current| !choice.same_model(current))
            {
                return Ok(replacement(shared, id, choice, reply.result["runtime"].clone()).await);
            }
            // A 3.1.4 model view alone cannot bind a newly saved choice to its backend.
            // A missing or changing config-fields identity takes the existing visible fallback.
            let Some(confirmed) = confirmed.filter(|choice| !choice.provider_key.is_empty()) else {
                let raw = super::actual(&reply.result);
                let choice = selected.as_ref().or(raw.as_ref());
                return if let Some(choice) = choice {
                    Ok(replacement(shared, id, choice, reply.result["runtime"].clone()).await)
                } else {
                    Err(CoreError::bad_result(method, "runtime.config.activeModel"))
                };
            };
            let outcome = super::accepted(&reply.result)
                .and_then(|()| shared.model_choices.accept_pick(id, params, confirmed));
            if let Err(error) = outcome {
                if error.code == crate::error::ErrorCode::Internal {
                    // Vibe has already changed the in-memory model. If persisting the pick fails,
                    // put the model that was running before the write back into this chat.
                    if let Some(previous) = super::actual(&json!({"runtime":runtime})) {
                        match super::apply(shared, id, &previous).await {
                            Ok(restored) => publish(shared, id, restored).await,
                            Err(rollback_error) => {
                                tracing::warn!(%rollback_error, "previous chat model could not be restored after a failed save");
                            }
                        }
                    }
                    return Err(error);
                }
                if let Some(choice) = selected.as_ref() {
                    return Ok(
                        replacement(shared, id, choice, reply.result["runtime"].clone()).await,
                    );
                }
                return Err(error);
            }
            publish(shared, id, reply.result["runtime"].clone()).await;
            Ok(reply)
        }
        Err(error) => {
            // A refused model pick uses the same notice as a model missing at reopen.
            if let (Some(choice), Ok(runtime)) =
                (selected.as_ref(), super::restore::read(shared, id).await)
            {
                return Ok(replacement(shared, id, choice, runtime).await);
            }
            Err(CoreError::from_gate(method, &error))
        }
    }
}

async fn replacement(shared: &Arc<Shared>, id: &str, choice: &Choice, runtime: Value) -> Reply {
    let runtime = super::restore::fallback(shared, id, choice, runtime).await;
    publish(shared, id, runtime.clone()).await;
    Reply {
        result: json!({"runtime":runtime,"modelChoiceFallback":true,"failures":[]}),
        client_user_message_id: None,
        user_entry_ids: vec![],
    }
}

pub(super) fn selection(params: &Value, runtime: &Value) -> Option<Choice> {
    let alias = params["modelAlias"].as_str()?;
    let row = runtime
        .pointer("/config/models")?
        .as_array()?
        .iter()
        .find(|row| row["alias"] == alias)?;
    let mut choice = actual(&json!({"runtime":{"config":{"activeModel":row}}}))?;
    if let Some(effort) = params["reasoningEffort"].as_str() {
        choice.effort = effort.into();
    }
    Some(choice)
}

pub(crate) fn selected(shared: &Shared, method: &str, params: &mut Value) -> Option<Choice> {
    if method != "config/model/write" {
        return None;
    }
    let runtime = params["sessionId"]
        .as_str()
        .and_then(|id| shared.store.runtime(id));
    let fields = params.as_object_mut()?;
    let name = fields.remove("_modelName");
    let display = fields.remove("_modelDisplayName");
    let selected = runtime.and_then(|runtime| selection(params, &runtime));
    selected.or_else(|| {
        Some(Choice {
            alias: params["modelAlias"].as_str()?.into(),
            effort: params["reasoningEffort"].as_str().unwrap_or("off").into(),
            name: name
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(256)
                .collect(),
            provider: String::new(),
            provider_key: String::new(),
            display_name: display
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(256)
                .collect(),
        })
    })
}
