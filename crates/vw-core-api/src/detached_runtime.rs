//! Saved composer selections over the lobby's plugin-free configuration for detached chat reads.
//! Called by `detached_reads`; a display snapshot is never published as an attached runtime.
use crate::{core::Shared, error::CoreError};
use serde_json::{Value, json};

pub(crate) async fn config(shared: &Shared, state: &Value) -> Result<Value, CoreError> {
    // A lobby read for the saved folder, so no channel opens for the stored chat. It still loads the plugins
    // root, and it answers the chat's own config/read (which `loads_plugins` sweeps for): sweep first, so the
    // server never sees an unapproved plugin folder (vw_plugin_boundaries).
    shared.channels.sweep_plugins().await?;
    let params = state["session"]["cwd"]
        .as_str()
        .map_or_else(|| json!({}), |cwd| json!({"cwd":cwd}));
    let mut config = shared
        .gate
        .call("config/read", params, None)
        .await
        .map_err(|e| CoreError::from_gate("config/read", &e))?
        .result["config"]
        .clone();
    let session = &state["session"];
    let name = match (session["id"].as_str(), session["model"].as_str()) {
        (Some(id), Some(alias)) => shared.model_choices.saved_name(id, alias),
        _ => String::new(),
    };
    saved_model(&mut config, session, &name);
    Ok(config)
}

/// Shows the chat's saved model (`session.model`, `session.reasoningEffort`) as the config view's pinned active
/// model: its row from the list; for Vibe Work's retired `large-4` alias, the one catalog Large 4 the chat reopens
/// on (`model_choices::retired_row`) unless the chat's saved choice (`name`) names another model; else a row named
/// by the alias alone.
fn saved_model(config: &mut Value, session: &Value, name: &str) {
    let Some(alias) = session["model"].as_str() else {
        return;
    };
    let effort = session["reasoningEffort"].as_str();
    let listed = config["models"]
        .as_array()
        .and_then(|models| models.iter().find(|model| model["alias"] == alias))
        .cloned();
    if listed.is_none()
        && let Some(row) =
            crate::model_choices::retired_row(config, alias, effort.unwrap_or_default(), name)
    {
        config["activeModel"] = row;
        config["activeModelPinned"] = json!(true);
        return;
    }
    config["activeModel"] = listed.unwrap_or_else(|| {
        json!({
            "alias":alias,"name":alias,"displayName":alias,
            "thinking":effort.unwrap_or_default(),"supportsImages":false
        })
    });
    config["activeModelPinned"] = json!(true);
    if let Some(effort) = effort {
        config["activeModel"]["thinking"] = json!(effort);
    }
}

pub(crate) async fn read(shared: &Shared, id: &str, state: &Value) -> Result<Value, CoreError> {
    if let Some(runtime) = shared.store.runtime(id) {
        return Ok(runtime);
    }
    let config = config(shared, state).await?;
    let agent = state["session"].get("agent").filter(|agent| agent.is_object()).cloned()
        .unwrap_or_else(|| {
            let name = crate::restore::display_mode(shared, id, &state["session"])
                .unwrap_or_else(|| crate::access::FALLBACK_PROFILE.into());
            json!({"name":name,"displayName":name,"description":"","safety":"neutral","agentType":"agent"})
        });
    let context_window = config["activeModel"]["maxContextLength"]
        .as_i64()
        .unwrap_or(0);
    Ok(
        json!({"config":config,"activeAgent":agent,"agents":[agent],"skills":[],"tools":[],
        "stats":{},"contextWindow":context_window,"issues":[],"hooksCount":0,"connectors":{},"mcp":{}}),
    )
}

#[cfg(test)]
mod tests {
    use super::saved_model;
    use serde_json::{Value, json};

    fn lobby(rows: &[Value]) -> Value {
        json!({"models":rows,"activeModel":rows[0].clone()})
    }

    fn large() -> Value {
        json!({"alias":"mistral-large-4-0","name":"mistral-large-4-0","source":"catalog",
            "displayName":"Mistral Large 4","thinking":"high","thinkingLevels":["low","medium","high"]})
    }

    fn medium() -> Value {
        json!({"alias":"mistral-medium-latest","name":"mistral-medium-latest","source":"catalog",
            "displayName":"Mistral Medium 3.5","thinking":"high","thinkingLevels":["off","high"]})
    }

    /// A chat Vibe still records on the retired `large-4` alias (not reopened since the alias was retired)
    /// shows the catalog Large 4 it reopens on, with its own level, so the chip names it and the menu checks it.
    ///
    /// PREDICT (written before the change): the alias is not in the list, so the view's active model is a made-up
    /// row `{alias:"large-4", displayName:"large-4"}`: the chip reads the raw "large-4" and no menu item is checked.
    #[test]
    fn a_detached_chat_on_the_retired_alias_shows_the_catalog_large_4() {
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"medium"}),
            "",
        );
        let active = &config["activeModel"];
        assert_eq!(active["alias"], "mistral-large-4-0", "{config}");
        assert_eq!(active["displayName"], "Mistral Large 4", "{config}");
        assert_eq!(active["thinking"], "medium", "{config}");
        assert_eq!(config["activeModelPinned"], true);
        // A level never set (Off) shows the model's own level: Large 4 offers no Off.
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"off"}),
            "",
        );
        assert_eq!(config["activeModel"]["thinking"], "high", "{config}");
        // The chat's saved choice is a project's own model under the alias (a private provider's): not Large 4.
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"high"}),
            "private-local-model",
        );
        assert_eq!(config["activeModel"]["alias"], "large-4", "{config}");
        assert_eq!(config["activeModel"]["displayName"], "large-4", "{config}");
        // The app seed's own record still shows the catalog Large 4.
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"high"}),
            "mistral-large-4",
        );
        assert_eq!(
            config["activeModel"]["alias"], "mistral-large-4-0",
            "{config}"
        );
    }

    /// Controls: a listed model shows its own row with the saved level; the person's own `large-4` row stays theirs;
    /// any other model that left the list, or a list that does not settle Large 4, keeps the alias-named row.
    #[test]
    fn other_saved_models_keep_their_rows() {
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"mistral-medium-latest","reasoningEffort":"off"}),
            "",
        );
        assert_eq!(config["activeModel"]["displayName"], "Mistral Medium 3.5");
        assert_eq!(config["activeModel"]["thinking"], "off");

        let own = json!({"alias":"large-4","name":"mistral-large-4","source":"local","displayName":"My Large 4"});
        let mut config = lobby(&[own, large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"high"}),
            "",
        );
        assert_eq!(config["activeModel"]["displayName"], "My Large 4");

        let gone = |rows: &[Value], alias: &str| {
            let mut config = lobby(rows);
            saved_model(
                &mut config,
                &json!({"model":alias,"reasoningEffort":"high"}),
                "",
            );
            config["activeModel"].clone()
        };
        let mut other = large();
        other["alias"] = json!("mistral-large-4");
        other["name"] = json!("mistral-large-4");
        for (rows, alias) in [
            (vec![medium(), large()], "retired-flash"),
            (vec![medium(), large(), other], "large-4"),
            (vec![medium()], "large-4"),
        ] {
            let active = gone(&rows, alias);
            assert_eq!(active["alias"], alias, "{active}");
            assert_eq!(active["displayName"], alias, "{active}");
        }
    }
}
