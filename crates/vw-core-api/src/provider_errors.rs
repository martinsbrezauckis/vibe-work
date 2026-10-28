//! Scrub stored secrets from provider failure details before desktop replies/events leave the core.
//! Uses vw-secrets' registry (also used by the Debug API); snapshots and session projections call this.
use serde_json::Value;

pub(crate) fn value(value: &mut Value) {
    // A failed session's status repeats its turn error; prefixless validation bodies live here too.
    if value.get("type").and_then(Value::as_str) == Some("failed") {
        scrub(value);
        return;
    }
    if value
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| {
            path == "/status/message" || path == "/error" || path.starts_with("/error/")
        })
    {
        if let Some(body) = value.get_mut("value") {
            scrub(body);
        }
        return;
    }
    match value {
        Value::String(text) if is_provider_error(text.trim()) => {
            if let Some((bytes, _)) = vw_secrets::scrub(text.as_bytes()) {
                *text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(self::value),
        Value::Object(fields) => {
            for (name, field) in fields {
                // HTTPValidationError has only a JSON body, with no SDK prefix. Error fields remain scrubbed
                // even when the UI cannot safely classify their wording.
                if name == "error" {
                    scrub(field);
                } else {
                    self::value(field);
                }
            }
        }
        _ => {}
    }
}

fn is_provider_error(text: &str) -> bool {
    [
        "API error occurred: Status ",
        "Unexpected response received: Status ",
        "LLM backend error [",
        "Invalid API key",
        "Rate limit exceeded",
        "Invalid model",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

pub(crate) fn event(event: &mut crate::CoreEvent) {
    use crate::event::{CoreEvent, SessionChange};
    match event {
        CoreEvent::Snapshot { snapshot } => value(&mut snapshot.state),
        CoreEvent::Change { change } => match change {
            SessionChange::EntryUpserted { entry, .. } => value(entry),
            SessionChange::TurnUpserted { turn, .. } => value(turn),
            SessionChange::Notice { params, .. } => value(params),
            SessionChange::ChildUpserted { child, .. } => value(child),
            SessionChange::SessionPatched { patch, .. }
            | SessionChange::EntryPatched { patch, .. } => patch.iter_mut().for_each(value),
            _ => {}
        },
        _ => {}
    }
}

fn scrub(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some((bytes, _)) = vw_secrets::scrub(text.as_bytes()) {
                *text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(scrub),
        Value::Object(fields) => fields.values_mut().for_each(scrub),
        _ => {}
    }
}
