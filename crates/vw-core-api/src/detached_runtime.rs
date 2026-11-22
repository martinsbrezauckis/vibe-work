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
    let (name, provider) = match (session["id"].as_str(), session["model"].as_str()) {
        (Some(id), Some(alias)) => shared.model_choices.saved_identity(id, alias),
        _ => (String::new(), String::new()),
    };
    saved_model(&mut config, session, &name, &provider);
    Ok(config)
}

/// Shows the chat's saved model (`session.model`, `session.reasoningEffort`) as the config view's pinned active
/// model: its row from the list; for Vibe Work's retired `large-4` alias, the one catalog Large 4 the chat reopens
/// on (`model_choices::retired_row`) unless the chat's saved choice (`name`) names another model; else a row named
/// by the alias alone.
fn saved_model(config: &mut Value, session: &Value, name: &str, provider: &str) {
    let Some(alias) = session["model"].as_str() else {
        return;
    };
    let effort = session["reasoningEffort"].as_str();
    let saved_identity = !name.is_empty() || !provider.is_empty();
    // A 3.1.4 config view cannot prove a saved provider; do not present a current
    // same-name row as the detached chat's saved backend.
    let listed = (!saved_identity)
        .then(|| {
            config["models"]
                .as_array()?
                .iter()
                .find(|model| model["alias"] == alias)
                .cloned()
        })
        .flatten();
    if listed.is_none()
        && let Some(row) = crate::model_choices::retired_row(
            config,
            alias,
            effort.unwrap_or_default(),
            name,
            provider,
        )
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
    let session = &state["session"];
    let policy = crate::restore::display_mode(shared, id, session)
        .unwrap_or_else(|| crate::access::FALLBACK_PROFILE.into());
    Ok(detached_snapshot(&config, session, &policy))
}

/// Builds the saved display snapshot; the detached path never opens the stored chat's channel.
fn detached_snapshot(config: &Value, session: &Value, policy: &str) -> Value {
    let agent = actual_agent(config, session);
    let active_policy = policy_summary(policy);
    // A detached chat has no current capability list. These three policies are always available;
    // Smart is included only when a live runtime advertises it, never by this projection.
    let policies = ["ask", "accept-edits", "auto-approve"].map(policy_summary);
    let context_window = config["activeModel"]["maxContextLength"]
        .as_i64()
        .unwrap_or(0);
    json!({"config":config,"activeAgent":agent,"agents":[agent],
        "activeApprovalPolicy":active_policy,"approvalPolicies":policies,"skills":[],"tools":[],
        "stats":{},"contextWindow":context_window,"issues":[],"hooksCount":0,"connectors":{},"mcp":{}})
}

fn actual_agent(config: &Value, session: &Value) -> Value {
    let saved = session.get("agent").filter(|agent| agent.is_object());
    let saved_name = saved.and_then(|agent| agent["name"].as_str());
    let name = saved_name
        .filter(|name| is_agent_name(name))
        .or_else(|| {
            config["defaultAgent"]
                .as_str()
                .filter(|name| is_agent_name(name))
        })
        .unwrap_or("default");
    let mut agent = if saved_name == Some(name) {
        saved.cloned().unwrap_or_default()
    } else {
        json!({"name":name,"displayName":name,"description":""})
    };
    if let Some(agent) = agent.as_object_mut() {
        agent.remove("safety"); // retired AgentSummary field; safety belongs to ApprovalPolicySummary.
        agent.entry("displayName").or_insert_with(|| json!(name));
        agent.entry("description").or_insert_with(|| json!(""));
        agent.entry("agentType").or_insert_with(|| json!("agent"));
    }
    agent
}

fn is_agent_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(
            name,
            "ask" | "accept-edits" | "auto-approve" | "smart-approve" | "vw-plan"
        )
}

/// Safety values from the 3.1.4 built-ins in `vibe/core/agents/models.py`; display text is shared
/// with the rest of Vibe Work's access controls.
fn policy_summary(name: &str) -> Value {
    let safety = match name {
        "accept-edits" => "destructive",
        "auto-approve" => "yolo",
        "smart-approve" => "smart",
        _ => "neutral",
    };
    let term = vw_vocabulary::term("accessModes", name);
    let display_name = term.map_or(name, |term| term.name);
    let description = term.map_or("", |term| term.description);
    json!({"name":name,"displayName":display_name,"description":description,"safety":safety})
}

#[cfg(test)]
mod tests {
    use super::{detached_snapshot, saved_model};
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

    #[test]
    fn a_detached_choice_does_not_claim_a_same_alias_from_another_provider() {
        let mut catalog = large();
        catalog["alias"] = json!("large-4");
        catalog["name"] = json!("mistral-large-4");
        for saved_provider in ["llamacpp", ""] {
            let mut config = lobby(&[medium(), catalog.clone()]);
            saved_model(
                &mut config,
                &json!({"model":"large-4","reasoningEffort":"high"}),
                "mistral-large-4",
                saved_provider,
            );
            assert_eq!(
                config["activeModel"]["displayName"], "large-4",
                "saved provider {saved_provider}: {config}"
            );
        }
    }

    /// Minimal new `ConfigView` shape written from the generated 3.1.4 model schema.
    fn config_view() -> Value {
        json!({
            "activeModel": {"name":"mistral-large-4-0","alias":"mistral-large-4-0",
                "thinking":"high","supportsImages":true,"displayName":"Mistral Large 4",
                "maxContextLength":32000},
            "activeModelPinned":false,"defaultModelAlias":"mistral-large-4-0","theme":"dark",
            "logLevel":null,"disableWelcomeBannerAnimation":false,
            "autocopyToClipboard":false,"fileWatcherForAutocomplete":false,
            "askConfirmationOnExit":false,"voiceModeEnabled":false,"narratorEnabled":false,
            "showThinkingNodes":false,"showSubagentStatusList":false,"worktreeLimit":2,
            "enableUpdateChecks":true,"enableNotifications":true,
            "experimentalEnableTabStatus":false,"models":[],"transcribeModels":[],"ttsModels":[],
            "transcription":{"model":{"name":"transcribe","sampleRate":16000,"encoding":"pcm_s16le",
                "language":"en","targetStreamingDelayMs":0},
                "provider":{"apiBase":"","apiKeyEnvVar":"","client":"mistral"}},
            "speech":{"model":{"name":"tts","voice":"default","responseFormat":"wav"},
                "provider":{"apiBase":"","apiKeyEnvVar":"","client":"mistral"}},
            "validationWarnings":[],"defaultAgent":"default","defaultApprovalPolicy":"accept-edits"
        })
    }

    #[test]
    fn detached_runtime_separates_agent_from_policy_and_matches_vibe314_schema() {
        let config = config_view();
        let saved_custom = json!({"agent":{"name":"my-agent","displayName":"My agent",
            "description":"Private agent","safety":"safe"}});
        for policy in ["ask", "accept-edits", "auto-approve"] {
            let runtime = detached_snapshot(&config, &saved_custom, policy);
            assert_eq!(runtime["activeAgent"]["name"], "my-agent", "{runtime}");
            assert_eq!(runtime["activeAgent"]["agentType"], "agent", "{runtime}");
            assert!(runtime["activeAgent"].get("safety").is_none(), "{runtime}");
            assert_eq!(runtime["activeApprovalPolicy"]["name"], policy, "{runtime}");
            let offered = runtime["approvalPolicies"].as_array().expect("policy list");
            assert_eq!(
                offered
                    .iter()
                    .map(|row| row["name"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["ask", "accept-edits", "auto-approve"]
            );
            assert!(serde_json::from_value::<vw_protocol::RuntimeSnapshot>(runtime).is_ok());
        }
        let missing = detached_snapshot(&config, &json!({}), "ask");
        assert_eq!(missing["activeAgent"]["name"], "default");
        assert_eq!(missing["activeApprovalPolicy"]["name"], "ask");
        let legacy = detached_snapshot(
            &config,
            &json!({"agent":{"name":"ask","safety":"neutral"}}),
            "ask",
        );
        assert_eq!(legacy["activeAgent"]["name"], "default");
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
            "mistral-large-4",
            "mistral",
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
            "mistral-large-4",
            "mistral",
        );
        assert_eq!(config["activeModel"]["thinking"], "high", "{config}");
        // The chat's saved choice is a project's own model under the alias (a private provider's): not Large 4.
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"high"}),
            "private-local-model",
            "llamacpp",
        );
        assert_eq!(config["activeModel"]["alias"], "large-4", "{config}");
        assert_eq!(config["activeModel"]["displayName"], "large-4", "{config}");
        // The app seed's own record still shows the catalog Large 4.
        let mut config = lobby(&[medium(), large()]);
        saved_model(
            &mut config,
            &json!({"model":"large-4","reasoningEffort":"high"}),
            "mistral-large-4",
            "mistral",
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
            "",
        );
        assert_eq!(config["activeModel"]["displayName"], "My Large 4");

        let gone = |rows: &[Value], alias: &str| {
            let mut config = lobby(rows);
            saved_model(
                &mut config,
                &json!({"model":alias,"reasoningEffort":"high"}),
                "",
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
