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
    shared.access.observe_runtime(id, &runtime);
    let applied = shared.store.apply(vw_session::Notification::new(
        "runtime/updated",
        json!({"sessionId":id,"runtime":runtime}),
    ));
    let _ = shared.loads.send(applied).await;
}

pub(crate) async fn before(shared: &Arc<Shared>, method: &str, params: &Value) {
    if !matches!(
        method,
        "turn/start" | "session/turn/enqueue" | "session/turn/queue/resume"
    ) {
        return;
    }
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    let choices = &shared.model_choices;
    let expected = choices
        .fallbacks
        .get(id)
        .or_else(|| choices.pending.get(id))
        .or_else(|| choices.remembered.get(id));
    let runtime = match super::restore::read(shared, id).await {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::warn!(%error, "chat model could not be read before sending");
            return;
        }
    };
    let expected = expected.or_else(|| super::restore::unavailable_default(&runtime["config"]));
    let runtime =
        if let Some(choice) = expected.filter(|choice| super::restore::differs(&runtime, choice)) {
            super::restore::fallback(shared, id, &choice, runtime).await
        } else {
            runtime
        };
    publish(shared, id, runtime).await;
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
    if let Some(choice) = selected
        .as_ref()
        .filter(|choice| !super::restore::offered(&runtime, choice))
    {
        return Ok(replacement(shared, id, choice, runtime).await);
    }
    match shared.gate.call(method, params.clone(), None).await {
        Ok(reply) => {
            if let Some(choice) = selected.as_ref()
                && actual(&reply.result).is_some_and(|current| !choice.same_model(&current))
            {
                return Ok(replacement(shared, id, choice, reply.result["runtime"].clone()).await);
            }
            if let Err(error) = shared.model_choices.observe(method, params, &reply.result) {
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
