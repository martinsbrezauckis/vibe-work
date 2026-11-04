//! Host-side UI request contracts. The bridge validates before queueing; attribution can become read-only only
//! for a known route/action with its entire read payload validated. Unknown operations remain driving.
//! Depends on `serde_json` and core errors. Called by bridge and `ui_grants`; never trusts a page-supplied effect.

use serde_json::Value;
use vw_core_api::{CoreError, ErrorCode};

#[derive(Clone, Copy)]
enum Field {
    Text,
    NonEmptyText,
    Bool,
    Index,
    Duration,
}

impl Field {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Text => value.is_string(),
            Self::NonEmptyText => value.as_str().is_some_and(|s| !s.is_empty()),
            Self::Bool => value.is_boolean(),
            Self::Index => value.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991),
            Self::Duration => value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0),
        }
    }
}

/// A field's name, type and whether the caller must supply it.
type Fields = &'static [(&'static str, Field, bool)];
const SESSION: Fields = &[("sessionId", Field::NonEmptyText, false)];

/// Reads accept only the fields named in their contract. Inputs may ignore extra fields, but always drive.
enum Contract {
    Read(Fields),
    Input(Fields),
    Driving,
}

/// Explicit route/action pairs, alongside routes that have no actions. An action string alone has no authority.
fn contract(route: &str, action: Option<&str>) -> Contract {
    use Field::{Bool, Duration, Index, NonEmptyText, Text};
    match (route, action) {
        ("click", _) => {
            Contract::Input(&[("selector", NonEmptyText, true), ("index", Index, false)])
        }
        ("key", _) => Contract::Input(&[
            ("key", NonEmptyText, true),
            ("code", NonEmptyText, false),
            ("selector", NonEmptyText, false),
            ("index", Index, false),
            ("ctrl", Bool, false),
            ("shift", Bool, false),
            ("alt", Bool, false),
            ("meta", Bool, false),
        ]),
        ("type", _) => Contract::Input(&[("text", Text, true), ("selector", NonEmptyText, false)]),
        ("composer", Some("type")) => {
            Contract::Input(&[("text", Text, true), ("sessionId", NonEmptyText, false)])
        }
        ("controls", None) => Contract::Read(&[
            ("selector", NonEmptyText, false),
            ("includeHidden", Bool, false),
        ]),
        ("settle", None) => {
            Contract::Read(&[("quietMs", Duration, false), ("timeoutMs", Duration, false)])
        }
        ("rect", None) => Contract::Read(&[("selector", NonEmptyText, true)]),
        ("trace", None) => Contract::Read(&[("since", Index, false)]),
        ("secrets", None) => Contract::Read(&[("lock", Bool, false)]),
        ("state" | "revision" | "layout" | "setup", None)
        | ("toasts" | "notifications" | "walkthrough", None | Some("read"))
        | (
            "rail" | "sessions" | "settings" | "connectors" | "libraries" | "marketplace"
            | "models" | "updates" | "processes",
            Some("read"),
        )
        | (
            "menu" | "worktrees" | "projects" | "changes" | "explorer" | "skills" | "terminal",
            Some("state"),
        ) => Contract::Read(&[]),
        ("composer" | "documents" | "transcript" | "voice", Some("read"))
        | ("git" | "files" | "editors" | "project" | "pages" | "browser", Some("state")) => {
            Contract::Read(SESSION)
        }
        ("git-browser", Some("read")) => Contract::Read(&[("projectId", NonEmptyText, false)]),
        // No contract, an omitted action on a mixed route, or an unsupported action: it may act.
        _ => Contract::Driving,
    }
}

/// Validates known input/read payloads and says whether the request may drive the window. In particular, extra
/// fields on generic input cannot reduce attribution, and extra fields on a read cannot smuggle an input action.
pub(crate) fn drives(route: &str, body: &Value) -> Result<bool, CoreError> {
    let invalid = |message: String| {
        CoreError::new(ErrorCode::InvalidParams, format!("ui/{route}: {message}"))
    };
    let action = body.get("action").and_then(Value::as_str);
    let contract = contract(route, action);
    let (fields, driving) = match contract {
        Contract::Read(fields) => (fields, false),
        Contract::Input(fields) => (fields, true),
        Contract::Driving => return Ok(true),
    };
    // The read routes with no body accept null like the UI does. Input routes require an object.
    let empty = serde_json::Map::new();
    let input = if body.is_null() && !driving {
        &empty
    } else {
        body.as_object()
            .ok_or_else(|| invalid("body must be a JSON object".to_owned()))?
    };
    for (name, kind, required) in fields {
        match input.get(*name) {
            Some(value) if !kind.accepts(value) => return Err(invalid(format!("invalid {name}"))),
            None if *required => return Err(invalid(format!("missing {name}"))),
            _ => {}
        }
    }
    if !driving {
        for name in input.keys() {
            if name == "action" && action.is_some() {
                continue;
            }
            if !fields.iter().any(|(field, _, _)| name == field) {
                return Err(invalid(format!("unsupported field {name} for this read")));
            }
        }
    }
    Ok(driving)
}
