//! Carries saved model choices across a conversation's new session id.
//! Called by the shared handoff path for replies and notifications; the original saved chat keeps its records.
use std::sync::PoisonError;

use super::Choices;

/// Replies and notifications may announce the same move; copy its choices once.
pub(crate) fn handed_off(choices: &Choices, old: &str, new: &str) {
    // A reply and a later notification may announce the same move. A pick on the replacement
    // must survive that duplicate even though the original saved chat retains its own records.
    if !choices
        .handed_off
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((old.into(), new.into()))
    {
        return;
    }
    for records in [&choices.fallbacks, &choices.pending, &choices.remembered] {
        if let Some(choice) = records.get(old)
            && let Err(error) = records.change(new, Some(choice.clone()))
        {
            tracing::warn!(%error, "model choice could not be saved for the replacement chat");
            // Keep the choice in memory when its disk write fails.
            records
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(new.into(), choice);
        }
    }
}

/// A branch keeps the source choice, including one Vibe has not recorded at a turn yet.
pub(crate) fn forked(
    shared: &crate::core::Shared,
    params: &serde_json::Value,
    result: &serde_json::Value,
) {
    let Some(source) = params["sourceSessionId"].as_str() else {
        return;
    };
    let Some(id) = crate::routing::result_session(result) else {
        return;
    };
    let choices = &shared.model_choices;
    let choice = choices
        .fallbacks
        .get(source)
        .or_else(|| choices.pending.get(source))
        .or_else(|| choices.remembered.get(source))
        .or_else(|| {
            shared
                .store
                .view(source)
                .and_then(|view| super::recorded(&view.state["session"]))
        });
    if let Some(choice) = choice
        && let Err(error) = choices.pending.change(id, Some(choice))
    {
        tracing::warn!(%error, "branch model choice could not be saved");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_repeated_handoff_does_not_reinstate_a_replaced_choice() {
        let choices = Choices::load(None);
        let pick = |id: &str, alias: &str| {
            let effort = "off";
            choices.observe("config/model/write", &json!({"sessionId":id,"modelAlias":alias}),
                &json!({"runtime":{"config":{"activeModel":{"alias":alias,"name":alias,"thinking":effort}}}})).unwrap();
        };
        pick("old", "flash");
        handed_off(&choices, "old", "new");
        pick("new", "large");
        handed_off(&choices, "old", "new");
        assert_eq!(choices.pending.get("new").unwrap().alias, "large");
    }
}
