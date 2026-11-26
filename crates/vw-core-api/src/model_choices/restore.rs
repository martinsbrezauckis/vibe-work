//! Restores chat choices, falling back to Settings with one alert when a model is unavailable (every model alike: no
//! chat is moved to another entry without that alert).
//! Called by reopen and model selection; unavailable account lists preserve choices for a later Send.
use super::{Choice, actual, apply};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use serde_json::{Value, json};

/// The chat's model is in Vibe's model list, and so is its thinking level when Vibe lists the model's levels
/// (Vibe 3.0.3 reports only the levels it offers, e.g. Off and High for Medium 3.5, and sends a level as picked).
/// A choice that is not offered takes the same Settings fallback as a model that left the list.
pub(super) fn offered(runtime: &Value, choice: &Choice) -> bool {
    runtime
        .pointer("/config/models")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models.iter().any(|model| {
                let level_offered = choice.effort.is_empty()
                    || model["thinkingLevels"].as_array().is_none_or(|levels| {
                        levels
                            .iter()
                            .any(|level| level.as_str() == Some(&choice.effort))
                    });
                level_offered
                    && model["alias"] == choice.alias
                    && (choice.name.is_empty() || model["name"] == choice.name)
            })
        })
}

/// An absent alias cannot retire a saved choice while the account's list is unavailable.
pub(super) fn waiting_for_account_models(
    shared: &Shared,
    runtime: &Value,
    choice: &Choice,
) -> bool {
    crate::model_check::account_models_unavailable(shared, &runtime["config"])
        && !offered(runtime, choice)
}

fn saved_choice_waits_for_account_models(
    shared: &Shared,
    runtime: &Value,
    prior: &super::records::Record,
    recorded: Option<&Choice>,
) -> bool {
    prior
        .expected()
        .iter()
        .chain(recorded)
        .any(|choice| waiting_for_account_models(shared, runtime, choice))
}

/// A catalog row can be absent from the first runtime after resume while Vibe's resolved fields
/// already contain it. Only a saved, backend-bound identity may use that row for replay.
pub(super) async fn replayable(
    shared: &Shared,
    id: &str,
    runtime: &Value,
    choice: &Choice,
) -> bool {
    if offered(runtime, choice) {
        return true;
    }
    if choice.name.is_empty() || choice.provider.is_empty() || choice.provider_key.is_empty() {
        return false;
    }
    let mut probe = choice.clone();
    probe.provider.clear();
    probe.provider_key.clear();
    let resolved = super::identified(shared, id, probe).await;
    if resolved.provider_key.is_empty() || !choice.matches(&resolved) {
        return false;
    }
    // The gate learns permitted aliases only from runtime/read. A resolved fields row alone
    // cannot authorize config/model/write; after catalog hydration the runtime must offer it.
    read(shared, id)
        .await
        .is_ok_and(|refreshed| offered(&refreshed, choice))
}

/// The same alias now resolves to a different backend. This is positive evidence for the
/// existing unavailable-choice notice, unlike a failed write or an incomplete runtime.
pub(super) async fn backend_changed(shared: &Shared, id: &str, choice: &Choice) -> bool {
    if choice.provider_key.is_empty() {
        return false;
    }
    let mut probe = choice.clone();
    probe.provider.clear();
    probe.provider_key.clear();
    let resolved = super::identified(shared, id, probe).await;
    !resolved.provider_key.is_empty() && resolved.provider_key != choice.provider_key
}

/// A failed cold write may wait only when the usable catalog still proves the saved backend.
/// Missing resolved fields are not proof that an older saved digest names the current route.
async fn backend_matches(shared: &Shared, id: &str, choice: &Choice) -> bool {
    let mut probe = choice.clone();
    probe.provider.clear();
    probe.provider_key.clear();
    let resolved = super::identified(shared, id, probe).await;
    !resolved.provider_key.is_empty() && choice.matches(&resolved)
}

pub(super) fn mark_cold_replay(
    shared: &Shared,
    id: &str,
    runtime: &Value,
    expected: Option<&Choice>,
) {
    if let Some(choice) =
        expected.filter(|choice| !choice.provider_key.is_empty() && !offered(runtime, choice))
    {
        shared.model_choices.mark_cold_replay(id, choice.clone());
    }
}

/// Vibe resolves catalog aliases before reporting a missing or excluded default.
pub(super) fn unavailable_default(config: &Value) -> Option<Choice> {
    let alias = config["validationWarnings"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|warning| {
            warning
                .strip_prefix("Model '")
                .and_then(|text| {
                    text.split_once("' is not available")
                        .map(|(alias, _)| alias)
                })
                .or_else(|| {
                    warning.strip_prefix("Active model '").and_then(|text| {
                        text.split_once("' is excluded by allowed_models")
                            .map(|(alias, _)| alias)
                    })
                })
        })?;
    Some(Choice {
        alias: alias.into(),
        effort: String::new(),
        name: String::new(),
        provider: String::new(),
        provider_key: String::new(),
        display_name: String::new(),
    })
}

async fn default_choice(shared: &Shared, choice: &mut Choice) -> Result<(Choice, bool), CoreError> {
    // The lobby reports Settings without a chat's older turn pins or agent layer.
    let reply = shared
        .gate
        .call("config/read", json!({}), None)
        .await
        .map_err(|error| CoreError::from_gate("config/read", &error))?;
    if choice.display_name.is_empty()
        && let Some(row) =
            super::live::selection(&json!({"modelAlias":choice.alias}), &reply.result)
    {
        choice.display_name = row.display_name;
        if choice.effort.is_empty() {
            choice.effort = row.effort;
        }
    }
    shared.model_choices.learn(&reply.result);
    if let Some(missing) = unavailable_default(&reply.result["config"]) {
        return Ok((shared.model_choices.named(missing), false));
    }
    actual(&json!({"runtime":reply.result}))
        .map(|choice| (choice, true))
        .ok_or_else(|| CoreError::bad_result("config/read", "config.activeModel"))
}

pub(super) async fn read(shared: &Shared, id: &str) -> Result<Value, CoreError> {
    shared
        .gate
        .call("runtime/read", json!({"sessionId":id}), None)
        .await
        .map(|reply| reply.result["runtime"].clone())
        .map_err(|error| CoreError::from_gate("runtime/read", &error))
}

/// A listed model is not proof that Vibe's first-turn catalog work has finished. Wait before
/// deciding whether to replay a bound choice; a catalog request refused while locked must also
/// be refreshed with the now-usable key before an absent account row can retire the choice.
pub(super) async fn after_ready(
    shared: &Shared,
    id: &str,
    last: Value,
    choice: &Choice,
) -> Result<(Value, bool), CoreError> {
    if choice.name.is_empty()
        || choice.provider.is_empty()
        || choice.provider_key.is_empty()
        || !super::settled::key_usable(shared)
    {
        return Ok((last, false));
    }
    let cold_missing = !offered(&last, choice);
    let mut runtime = ready_runtime(shared, id).await?;
    if !offered(&runtime, choice)
        && !runtime["config"]["models"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| matches!(row["source"].as_str(), Some("catalog" | "growthbook")))
        })
    {
        // Vibe holds a failed first fetch in its process cache. A ready flag after that failure
        // does not mean that the account's list was fetched with the key unlocked in the app.
        let method = "config/reload";
        shared
            .gate
            .call(method, json!({"sessionId":id,"reloadRuntime":true}), None)
            .await
            .map_err(|error| CoreError::from_gate(method, &error))?;
        runtime = ready_runtime(shared, id).await?;
    }
    Ok((runtime, cold_missing))
}

async fn ready_runtime(shared: &Shared, id: &str) -> Result<Value, CoreError> {
    let method = "runtime/read";
    let reply = shared
        .gate
        .call(method, json!({"sessionId":id}), None)
        .await
        .map_err(|error| CoreError::from_gate(method, &error))?;
    if reply.result["ready"] != false {
        return Ok(reply.result["runtime"].clone());
    }
    let method = "session/ready/wait";
    let ready = shared
        .gate
        .call(method, json!({"sessionId":id}), None)
        .await
        .map_err(|error| CoreError::from_gate(method, &error))?;
    if ready.result["ready"] != true {
        return Err(CoreError::new(
            ErrorCode::Timeout,
            "The saved model for this chat is still loading. Try Send again.",
        ));
    }
    read(shared, id).await
}

pub(super) async fn fallback(shared: &Shared, id: &str, choice: &Choice, last: Value) -> Value {
    let old_identity_ambiguous = choice.provider.is_empty()
        && !choice.name.is_empty()
        && last
            .pointer("/config/models")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row["alias"] == choice.alias
                        && row["name"] == choice.name
                        && row["source"] == "project"
                })
            });
    let why = if old_identity_ambiguous {
        "could not be identified from its older saved choice"
    } else if offered(&last, choice) {
        "could not be restored"
    } else {
        "is unavailable right now"
    };
    let mut choice = choice.clone();
    let default = default_choice(shared, &mut choice).await;
    let mut runtime = last;
    let (default, problem) = match default {
        Ok((default, true)) if offered(&runtime, &default) => {
            match apply(shared, id, &default).await {
                Ok(applied) => {
                    runtime = applied;
                    (Some(default), None)
                }
                Err(error) => {
                    tracing::warn!(%error, "Settings default model could not be applied");
                    runtime = read(shared, id).await.unwrap_or(runtime);
                    (Some(default), Some("could not be applied"))
                }
            }
        }
        Ok((default, _)) => {
            runtime = read(shared, id).await.unwrap_or(runtime);
            (Some(default), Some("is unavailable right now"))
        }
        Err(error) => {
            tracing::warn!(%error, "Settings default model could not be read");
            runtime = read(shared, id).await.unwrap_or(runtime);
            (None, Some("could not be read"))
        }
    };
    let message = notice(&choice, default.as_ref(), &runtime, why, problem);
    // Remember what runs until Vibe records the next turn, so the older pin does not repeat the alert.
    if let Some(current) = super::identified_actual(shared, id, &json!({"runtime":runtime}))
        .await
        .filter(|choice| !choice.provider_key.is_empty())
        && let Err(error) = shared
            .model_choices
            .records
            .update(id, |record| record.replaced(current))
    {
        tracing::warn!(%error, "replacement model could not be saved");
    }
    shared.emit(crate::CoreEvent::Alert {
        session_id: Some(id.into()),
        kind: crate::AlertKind::ModelChoiceNotRestored,
        message,
    });
    runtime
}

fn same_label(left: &Choice, right: &Choice) -> bool {
    if left.provider != right.provider {
        return false;
    }
    left.alias == right.alias
        || (!left.display_name.is_empty() && left.display_name == right.display_name)
}

fn notice(
    choice: &Choice,
    default: Option<&Choice>,
    runtime: &Value,
    why: &str,
    problem: Option<&str>,
) -> String {
    let current = actual(&json!({"runtime":runtime}));
    let current_label = current
        .as_ref()
        .map_or_else(|| "Vibe's current model".into(), Choice::label);
    let chosen_runs = current
        .as_ref()
        .is_some_and(|current| same_label(choice, current));
    if problem.is_none() {
        return if chosen_runs {
            format!(
                "This chat could not restore its model settings and now uses your default model, {current_label}."
            )
        } else {
            format!(
                "{} {why}, so this chat now uses your default model, {current_label}.",
                choice.label()
            )
        };
    }
    let default_runs = default
        .zip(current.as_ref())
        .is_some_and(|(default, current)| same_label(default, current));
    let default = default.map(|default| {
        if default.display_name.is_empty() && same_label(default, choice) {
            choice
        } else {
            default
        }
    });
    let label = default
        .filter(|default| !default.display_name.is_empty() && !default_runs)
        .map_or_else(
            || "Your Settings default model".into(),
            |default| format!("Your Settings default model, {},", default.label()),
        );
    let problem = problem.unwrap_or("could not be applied");
    let unavailable = if chosen_runs || default.is_some_and(|default| same_label(choice, default)) {
        format!("{label} {problem}.")
    } else {
        format!(
            "{} {why}, and {} {problem}.",
            choice.label(),
            label.replacen("Your", "your", 1)
        )
    };
    format!("{unavailable} This chat now uses {current_label}.")
}

/// Pending/fallback picks and proven remembered choices take the same replay path.
pub(super) fn saved_replay(record: &super::records::Record) -> Option<Choice> {
    record.replay().or_else(|| {
        record
            .remembered
            .clone()
            .filter(|choice| !choice.provider_key.is_empty())
    })
}

fn already_restored(
    record: &super::records::Record,
    runtime: &Value,
    choice: &Choice,
    current: Option<&Choice>,
) -> bool {
    // A pending pick also needs a current pin and an offered level before a read may skip its write.
    (record.fallback.is_some()
        || record.pending.is_none()
        || (runtime["config"]["activeModelPinned"] == true && offered(runtime, choice)))
        && current.is_some_and(|current| choice.matches(current))
}

/// Restoring a model never closes the chat or prevents a later Send.
pub(crate) async fn restore(
    shared: &Shared,
    id: &str,
    last: Value,
    recorded: Option<Choice>,
) -> Value {
    // Vibe's pin holds only the alias. Preserve the pre-migration identity: migration can
    // rewrite the saved seed's alias before this pin is interpreted.
    let prior = shared.model_choices.records.get(id);
    mark_cold_replay(shared, id, &last, prior.expected().as_ref());
    let (last, cold_missing) = if let Some(choice) = prior.expected() {
        match after_ready(shared, id, last.clone(), &choice).await {
            Ok(runtime) => runtime,
            Err(error) => {
                // A failed readiness check is not evidence that the person's model vanished.
                // Keep the saved choice for the next Send, which checks again before sending.
                tracing::warn!(%error, "saved chat model is still loading");
                if let Some(choice) = prior.expected() {
                    shared.model_choices.mark_cold_replay(id, choice);
                }
                return last;
            }
        }
    } else {
        (last, false)
    };
    if saved_choice_waits_for_account_models(shared, &last, &prior, recorded.as_ref()) {
        return last;
    }
    super::retired::migrate(&shared.model_choices.records, id, &last);
    let record = shared.model_choices.records.get(id);
    let current = super::identified_actual(shared, id, &json!({"runtime":last})).await;
    if let Some(choice) = saved_replay(&record) {
        if already_restored(&record, &last, &choice, current.as_ref()) {
            shared.model_choices.clear_cold_replay(id);
            return last;
        }
        if replayable(shared, id, &last, &choice).await {
            match apply(shared, id, &choice).await {
                Ok(runtime) => {
                    shared.model_choices.clear_cold_replay(id);
                    return runtime;
                }
                Err(error)
                    if (cold_missing || shared.model_choices.cold_replay_matches(id, &choice))
                        && !choice.provider_key.is_empty()
                        && backend_matches(shared, id, &choice).await =>
                {
                    // A rejected write does not prove the bound model is gone. Keep the pick
                    // until a later Send can replay it or report the failure to the person.
                    tracing::warn!(%error, "saved chat model could not be applied yet");
                    return last;
                }
                Err(_) => {}
            }
        }
        shared.model_choices.clear_cold_replay(id);
        return fallback(shared, id, &choice, last).await;
    }
    // Vibe's own pin of the retired Large 4 alias names the catalog Large 4: put it back silently (this also ends
    // Vibe's missing-model warning, which lasts until the chat's pin changes), even when the default runs Large 4.
    // Vibe's pin names only the alias; the chat's saved choice of that alias says which model it is, so a project's
    // own `large-4` model is never taken for the retired app seed.
    let recorded = recorded.map(|mut pin| {
        if pin.name.is_empty()
            && let Some(saved) = prior
                .remembered
                .as_ref()
                .filter(|saved| saved.alias == pin.alias)
        {
            pin.name.clone_from(&saved.name);
            pin.provider.clone_from(&saved.provider);
            pin.provider_key.clone_from(&saved.provider_key);
        }
        pin
    });
    let retired = recorded
        .as_ref()
        .and_then(|choice| super::retired::resolve(&last, choice));
    let resolved = retired.is_some();
    let recorded = retired.or(recorded).map(|choice| {
        record
            .remembered
            .filter(|saved| saved.alias == choice.alias)
            .map_or(choice.clone(), |mut saved| {
                saved.effort = choice.effort;
                saved
            })
    });
    if resolved
        && let Some(choice) = recorded.as_ref()
        && replayable(shared, id, &last, choice).await
    {
        return match apply(shared, id, choice).await {
            Ok(runtime) => runtime,
            Err(_) => fallback(shared, id, choice, last).await,
        };
    }
    if let Some(choice) = recorded.filter(|choice| {
        differs(&last, choice, current.as_ref()) && !unset_off(&last, choice, current.as_ref())
    }) {
        if !offered(&last, &choice)
            && replayable(shared, id, &last, &choice).await
            && let Ok(runtime) = apply(shared, id, &choice).await
        {
            return runtime;
        }
        return fallback(shared, id, &choice, last).await;
    }
    last
}

/// A recorded Off that the chat's own model does not offer is no choice to restore: a chat whose level was left unset
/// records no level (read as Off, `model_choices::recorded`), and Vibe then runs the model's own default, as it does
/// for an Off it pinned for an unset level in older sessions. Large 4 offers no Off, so its chats reopen as they ran.
fn unset_off(runtime: &Value, choice: &Choice, current: Option<&Choice>) -> bool {
    choice.effort == "off"
        && !offered(runtime, choice)
        && current
            .cloned()
            .or_else(|| actual(&json!({"runtime":runtime})))
            .is_some_and(|current| choice.same_model(&current))
}

pub(super) fn differs(runtime: &Value, choice: &Choice, current: Option<&Choice>) -> bool {
    !offered(runtime, choice)
        || current
            .cloned()
            .or_else(|| actual(&json!({"runtime":runtime})))
            .is_some_and(|current| !choice.matches(&current))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(alias: &str, label: &str, effort: &str) -> Choice {
        Choice {
            alias: alias.into(),
            name: alias.into(),
            display_name: label.into(),
            effort: effort.into(),
            provider: String::new(),
            provider_key: String::new(),
        }
    }

    #[test]
    fn proven_choice_requires_current_backend_but_legacy_choice_waits_for_confirmation() {
        let runtime = json!({"config":{"activeModel":{"alias":"same","name":"same-name",
            "thinking":"route-level","displayName":"Same"},"models":[{"alias":"same",
            "name":"same-name","thinking":"route-level","displayName":"Same"}]}});
        let mut saved = model("same", "Same", "route-level");
        saved.name = "same-name".into();
        let legacy = saved.clone();
        saved.provider = "llamacpp".into();
        saved.provider_key = "old-route".into();
        let mut changed = saved.clone();
        changed.provider = "mistral".into();
        changed.provider_key = "new-route".into();
        assert!(differs(&runtime, &saved, Some(&changed)));
        assert!(
            differs(&runtime, &saved, None),
            "missing identity must not accept a proven choice"
        );
        assert!(
            !differs(&runtime, &legacy, Some(&changed)),
            "legacy records cannot infer a backend change from the 3.1.4 view"
        );
    }

    #[test]
    fn every_fallback_format_names_each_model_once_in_at_most_two_sentences() {
        let chosen = model("raw-chosen-id", "Chosen display", "off");
        let default = model("raw-default-id", "Default display", "high");
        let current = model("raw-current-id", "Current display", "high");
        let changed_thinking = model("raw-chosen-id", "Chosen display", "high");
        for default in [None, Some(&chosen), Some(&default), Some(&changed_thinking)] {
            for current in [&chosen, &current, &changed_thinking] {
                let runtime = json!({"config":{"activeModel":{
                    "alias":current.alias,"name":current.name,
                    "displayName":current.display_name,"thinking":current.effort
                }}});
                for problem in [
                    None,
                    Some("is unavailable right now"),
                    Some("could not be applied"),
                    Some("could not be read"),
                ] {
                    let message = notice(
                        &chosen,
                        default,
                        &runtime,
                        "is unavailable right now",
                        problem,
                    );
                    for label in ["Chosen display", "Default display", "Current display"] {
                        assert!(message.matches(label).count() <= 1, "{message}");
                    }
                    assert!(message.contains(&current.display_name), "{message}");
                    assert!(!message.contains("raw-"), "{message}");
                    assert!(message.split(". ").count() <= 2, "{message}");
                }
            }
        }
    }

    #[test]
    fn an_unknown_display_name_never_exposes_the_alias() {
        let missing = model("raw-missing-id", "", "off");
        let runtime = json!({"config":{"activeModel":{
            "alias":"raw-current-id","thinking":"high","displayName":"Current display"
        }}});
        assert_eq!(
            notice(
                &missing,
                Some(&missing),
                &runtime,
                "is unavailable right now",
                Some("is unavailable right now")
            ),
            "Your Settings default model is unavailable right now. This chat now uses Current display with high thinking."
        );
    }

    /// A Settings default Vibe no longer lists was "your Settings default model" without a name. Its
    /// display name from the last list that had it names it now (never the alias; unknown keeps the unnamed words).
    /// PREDICT (before the fix): the gone default's name is never learned, so the message stays unnamed.
    #[test]
    fn a_removed_default_is_named_from_the_last_list_that_had_it() {
        let choices = super::super::Choices::load(None);
        choices.learn(&json!({"config":{"models":[
            {"alias":"raw-flash-id","displayName":"GLM Flash"},
            {"alias":"raw-large-id","displayName":"Large 4"}
        ]}}));
        let mut gone = model("raw-flash-id", "", "");
        gone.name = String::new();
        let gone = choices.named(gone);
        let chosen = model("raw-chosen-id", "Chosen display", "off");
        let runtime = json!({"config":{"activeModel":{
            "alias":"raw-large-id","thinking":"high","displayName":"Large 4"
        }}});
        let message = notice(
            &chosen,
            Some(&gone),
            &runtime,
            "is unavailable right now",
            Some("is unavailable right now"),
        );
        assert_eq!(
            message,
            "Chosen display with thinking off is unavailable right now, and your Settings default model, GLM Flash, \
             is unavailable right now. This chat now uses Large 4 with high thinking."
        );
        let unknown = choices.named(model("raw-never-seen", "", ""));
        assert_eq!(unknown.display_name, "");
    }

    #[test]
    fn a_thinking_level_the_model_no_longer_offers_takes_the_settings_fallback() {
        let runtime = json!({"config":{"models":[
            {"thinking":"off","thinkingLevels":["off","high"],"alias":"large-4","name":"large-4"},
            {"thinking":"off","alias":"old","name":"old"}
        ]}});
        assert!(offered(&runtime, &model("large-4", "Large 4", "high")));
        assert!(!offered(&runtime, &model("large-4", "Large 4", "medium")));
        assert!(differs(&runtime, &model("large-4", "Large 4", "low"), None));
        // A model whose levels Vibe does not list keeps any level, as before.
        assert!(offered(&runtime, &model("old", "Old", "medium")));
    }

    #[test]
    fn an_off_the_running_model_does_not_offer_is_no_choice_to_restore() {
        let runtime = |alias: &str| {
            json!({"config":{
                "activeModel":{"thinking":"low","alias":alias,"name":alias},
                "models":[
                    {"thinking":"low","thinkingLevels":["low","medium","high"],"alias":"large-4","name":"large-4"},
                    {"thinking":"off","thinkingLevels":["off","high"],"alias":"flash","name":"flash"}
                ]
            }})
        };
        let unset = model("large-4", "", "off");
        assert!(differs(&runtime("large-4"), &unset, None));
        assert!(
            unset_off(&runtime("large-4"), &unset, None),
            "the chat runs as it ran"
        );
        // Another model running, an offered Off, or another level: the usual restore rules.
        assert!(!unset_off(&runtime("flash"), &unset, None));
        assert!(!unset_off(
            &runtime("flash"),
            &model("flash", "", "off"),
            None
        ));
        assert!(!unset_off(
            &runtime("large-4"),
            &model("large-4", "", "max"),
            None
        ));
    }
}
