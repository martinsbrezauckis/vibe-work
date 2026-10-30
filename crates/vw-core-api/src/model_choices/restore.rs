//! Restores chat choices, falling back to Settings with one alert when a model is unavailable (every model alike: no
//! chat is moved to another entry without that alert).
//! Called by reopen and model selection; failures leave the chat open and sendable.
use super::{Choice, actual, apply};
use crate::core::Shared;
use crate::error::CoreError;
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
                    && actual(&json!({"runtime":{"config":{"activeModel":model}}}))
                        .is_some_and(|current| choice.same_model(&current))
            })
        })
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
    if let Some(missing) = unavailable_default(&reply.result["config"]) {
        return Ok((missing, false));
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

pub(super) async fn fallback(shared: &Shared, id: &str, choice: &Choice, last: Value) -> Value {
    let why = if offered(&last, choice) {
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
    if let Some(current) = actual(&json!({"runtime":runtime})) {
        for records in [
            &shared.model_choices.fallbacks,
            &shared.model_choices.remembered,
        ] {
            if let Err(error) = records.change(id, Some(current.clone())) {
                tracing::warn!(%error, "replacement model could not be saved");
            }
        }
    }
    shared.model_choices.pending.forget(id);
    shared.emit(crate::CoreEvent::Alert {
        session_id: Some(id.into()),
        kind: crate::AlertKind::ModelChoiceNotRestored,
        message,
    });
    runtime
}

fn same_label(left: &Choice, right: &Choice) -> bool {
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

/// Restoring a model never closes the chat or prevents a later Send.
pub(crate) async fn restore(
    shared: &Shared,
    id: &str,
    last: Value,
    recorded: Option<Choice>,
) -> Value {
    let replacement = shared.model_choices.fallbacks.get(id);
    let is_replacement = replacement.is_some();
    if let Some(choice) = replacement.or_else(|| shared.model_choices.pending.get(id)) {
        if is_replacement
            && actual(&json!({"runtime":last})).is_some_and(|current| choice.matches(&current))
        {
            return last;
        }
        if offered(&last, &choice)
            && let Ok(runtime) = apply(shared, id, &choice).await
        {
            return runtime;
        }
        return fallback(shared, id, &choice, last).await;
    }
    let recorded = recorded.map(|choice| {
        shared
            .model_choices
            .remembered
            .get(id)
            .filter(|saved| saved.alias == choice.alias)
            .map_or(choice.clone(), |mut saved| {
                saved.effort = choice.effort;
                saved
            })
    });
    if let Some(choice) =
        recorded.filter(|choice| differs(&last, choice) && !unset_off(&last, choice))
    {
        return fallback(shared, id, &choice, last).await;
    }
    last
}

/// A recorded Off that the chat's own model does not offer is no choice to restore: a chat whose level was left unset
/// records no level (read as Off, `model_choices::recorded`), and Vibe then runs the model's own default, as it does
/// for an Off it pinned for an unset level in older sessions. Large 4 offers no Off, so its chats reopen as they ran.
fn unset_off(runtime: &Value, choice: &Choice) -> bool {
    choice.effort == "off"
        && !offered(runtime, choice)
        && actual(&json!({"runtime":runtime})).is_some_and(|current| choice.same_model(&current))
}

pub(super) fn differs(runtime: &Value, choice: &Choice) -> bool {
    !offered(runtime, choice)
        || actual(&json!({"runtime":runtime})).is_some_and(|current| !choice.same_model(&current))
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
        }
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

    #[test]
    fn a_thinking_level_the_model_no_longer_offers_takes_the_settings_fallback() {
        let runtime = json!({"config":{"models":[
            {"thinking":"off","thinkingLevels":["off","high"],"alias":"large-4","name":"large-4"},
            {"thinking":"off","alias":"old","name":"old"}
        ]}});
        assert!(offered(&runtime, &model("large-4", "Large 4", "high")));
        assert!(!offered(&runtime, &model("large-4", "Large 4", "medium")));
        assert!(differs(&runtime, &model("large-4", "Large 4", "low")));
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
        assert!(differs(&runtime("large-4"), &unset));
        assert!(
            unset_off(&runtime("large-4"), &unset),
            "the chat runs as it ran"
        );
        // Another model running, an offered Off, or another level: the usual restore rules.
        assert!(!unset_off(&runtime("flash"), &unset));
        assert!(!unset_off(&runtime("flash"), &model("flash", "", "off")));
        assert!(!unset_off(
            &runtime("large-4"),
            &model("large-4", "", "max")
        ));
    }
}
