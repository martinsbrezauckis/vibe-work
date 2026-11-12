//! The `browser_act` request: parsed from the tool's (or the Debug API's) JSON arguments into an
//! [`Action`], every field checked and bounded before anything reaches a page; and the act script's answer.
//!
//! Depends on: serde, `serde_json`, `model` (ref shape). Called by: `driver`, vw-core-api (tool and command
//! arguments), tests.
//! Invariants: a ref must have the ref shape; text to type is at most [`MAX_TEXT`] characters; keys are a fixed list
//! or one printable character; waits are at most [`MAX_WAIT_MS`].

use serde::Deserialize;
use serde_json::Value;

use crate::model::is_ref;

/// Longest text an action types.
pub const MAX_TEXT: usize = 10_000;
/// Longest wait an action asks for (ms), under Vibe's fixed 60 s tool timeout.
pub const MAX_WAIT_MS: u64 = 15_000;
/// Default wait (ms).
pub const DEFAULT_WAIT_MS: u64 = 5_000;
/// Named keys `press` accepts (plus any single printable character).
pub use vw_ask::chrome::keys::NAMED as KEYS;

#[path = "act_model.rs"]
mod model;
pub use model::{
    Action, Covering, MAX_UPLOAD_BYTES, MAX_UPLOAD_FILES, PageAnswer, Target, UploadFile,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Args {
    action: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    url: Option<String>,
    to: Option<String>,
    text: Option<String>,
    value: Option<String>,
    values: Option<Vec<String>>,
    checked: Option<bool>,
    key: Option<String>,
    direction: Option<String>,
    amount: Option<u32>,
    submit: Option<bool>,
    load: Option<bool>,
    timeout_ms: Option<u64>,
    paths: Option<Vec<String>>,
    file_ids: Option<Value>,
}

fn reference(value: Option<String>, action: &str) -> Result<String, String> {
    let text = value.ok_or_else(|| {
        format!("{action} needs a ref from browser_read (\"ref\": \"e1a2b3c4d\")")
    })?;
    if is_ref(&text) {
        Ok(text)
    } else {
        Err(format!(
            "'{}' is not a ref: use a ref from browser_read of this document (e + 8 hex digits)",
            text.chars().take(40).collect::<String>()
        ))
    }
}

fn selection(value: Option<String>, values: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let values = match (value, values) {
        (Some(value), None) => vec![value],
        (None, Some(values)) => values,
        _ => return Err("select needs exactly one of value or values; use values: [] to clear a multiple select".into()),
    };
    if values.len() > 100 || values.iter().any(|v| v.len() > 2000) {
        return Err(
            "select exceeds 100 options or 2000 bytes per option; choose fewer, shorter values"
                .into(),
        );
    }
    Ok(values)
}

fn optional_ref(value: Option<String>, action: &str) -> Result<Option<String>, String> {
    value.map(|v| reference(Some(v), action)).transpose()
}

/// `upload {ref, paths}`: 0 to [`MAX_UPLOAD_FILES`] chat-folder paths; the host loads the files.
fn upload(
    reference_arg: Option<String>,
    paths: Option<Vec<String>>,
    file_ids: bool,
) -> Result<Action, String> {
    if file_ids {
        return Err(
            "upload takes only \"paths\": files in this chat's folders (its attachments included)"
                .to_owned(),
        );
    }
    let paths = paths.ok_or("upload needs paths; use [] to clear this file selection")?;
    if paths.len() > MAX_UPLOAD_FILES {
        return Err(format!(
            "upload needs \"paths\": 0 to {MAX_UPLOAD_FILES} files in this chat's folders, relative to its folder \
             (for example [\"report.pdf\"])"
        ));
    }
    if paths.iter().any(|p| p.trim().is_empty() || p.len() > 4096) {
        return Err("each upload path must be a nonempty path of at most 4096 bytes".to_owned());
    }
    Ok(Action::Upload {
        reference: reference(reference_arg, "upload")?,
        paths,
        files: Vec::new(),
    })
}

#[path = "act_parse_groups.rs"]
mod parse_groups;

/// The actions `parse` knows, for its refusal of another name.
const ACTIONS: &str = "navigate, back, forward, reload, click, type, select, check, press, scroll, hover, drag, wait, upload";

/// Parses `browser_act` arguments.
///
/// # Errors
/// A plain-text reason the agent can act on.
pub fn parse(args: &Value) -> Result<Action, String> {
    let action = args["action"].as_str();
    if let Some(request) = match action {
        Some("recording.start") => Some(crate::recording::Request::Start),
        Some("recording.stop") => Some(crate::recording::Request::Stop),
        Some("recording.video.start") => Some(crate::recording::Request::VideoStart),
        Some("recording.video.stop") => Some(crate::recording::Request::VideoStop),
        _ => None,
    } {
        if args.get("format").is_some_and(|f| f != "gif") {
            return Err("Action recording supports GIF only; recording.video.start reports this engine's video availability.".into());
        }
        return Ok(Action::Recording(request));
    }
    if args["action"] == "tabs.audio" {
        return Ok(Action::TabAudio(None));
    }
    if args["action"] == "tabs.mute" {
        return args["muted"]
            .as_bool()
            .map(|m| Action::TabAudio(Some(m)))
            .ok_or_else(|| "Tab mute needs muted: true or false.".into());
    }
    if args["action"]
        .as_str()
        .is_some_and(|name| name.starts_with("media."))
    {
        return crate::playback::Request::parse(args).map(Action::Media);
    }
    if let Some(parsed) = parse_groups::developer(args) {
        return parsed;
    }
    if let Some(bypass) = args.get("bypassCache")
        && (args["action"] != "reload" || !bypass.is_boolean())
    {
        return Err("bypassCache must be a boolean on reload; omit it for other actions.".into());
    }
    if matches!(
        action,
        Some("console.read" | "page.errors" | "console.clear" | "page.errors.clear")
    ) {
        return crate::diagnostics::Request::parse(args).map(Action::Diagnostics);
    }
    if let Some(parsed) = parse_groups::input(args) {
        return parsed;
    }
    if matches!(
        action,
        Some(
            "page.click"
                | "move"
                | "wheel"
                | "page.double_click"
                | "page.triple_click"
                | "page.right_click"
                | "page.middle_click"
        )
    ) || args["action"] == "click" && args.get("modifiers").is_some()
    {
        return crate::pointer::parse(args);
    }
    if args["action"]
        .as_str()
        .is_some_and(|a| a.starts_with("page.dialog"))
    {
        return crate::dialogs::Request::parse(args).map(Action::Dialog);
    }
    if args["action"] == "capture.screenshot" {
        return crate::screenshot_area::Area::parse(args).map(Action::Screenshot);
    }
    if matches!(
        action,
        Some(
            "files.download"
                | "files.list_downloads"
                | "files.wait_download"
                | "files.save_download"
        )
    ) {
        return crate::downloads::Request::parse(args).map(Action::Downloads);
    }
    if args["action"]
        .as_str()
        .is_some_and(|name| name.starts_with("cookies."))
    {
        return vw_preview::CookieRequest::parse(args).map(Action::Cookies);
    }
    if args["action"]
        .as_str()
        .is_some_and(|name| name.starts_with("storage."))
    {
        return crate::site_storage::Request::parse(args).map(Action::Storage);
    }
    parse_page(args)
}

pub(crate) fn parse_page(args: &Value) -> Result<Action, String> {
    let a: Args = serde_json::from_value(args.clone())
        .map_err(|e| format!("browser_act arguments are not readable: {e}"))?;
    let action = a.action.unwrap_or_default();
    Ok(match action.as_str() {
        "navigate" => Action::Navigate {
            url: a
                .url
                .filter(|u| !u.trim().is_empty())
                .ok_or("navigate needs a url")?,
        },
        "back" => Action::Back,
        "forward" => Action::Forward,
        "reload" => reload(args),
        "click" if args.get("x").is_some() || args.get("y").is_some() => Action::ClickAt {
            reference: a
                .reference
                .map(|r| reference(Some(r), "click"))
                .transpose()?
                .unwrap_or_default(),
            x: coordinate(args, "x")?,
            y: coordinate(args, "y")?,
        },
        "click" => Action::Click {
            reference: reference(a.reference, "click")?,
        },
        "drag" => Action::Drag {
            reference: reference(a.reference, "drag")?,
            to: reference(a.to, "drag destination")?,
        },
        "hover" => Action::Hover {
            reference: reference(a.reference, "hover")?,
        },
        "type" | "fill" => {
            let text = a
                .text
                .or(a.value)
                .ok_or("type needs the text to type (\"text\")")?;
            if text.chars().count() > MAX_TEXT {
                return Err(format!("the text is longer than {MAX_TEXT} characters"));
            }
            Action::Type {
                reference: reference(a.reference, "type")?,
                text,
                submit: a.submit.unwrap_or(false),
            }
        }
        "select" => Action::Select {
            reference: reference(a.reference, "select")?,
            values: selection(a.value.or(a.text), a.values)?,
        },
        "check" | "uncheck" => Action::Check {
            reference: reference(a.reference, "check")?,
            checked: a.checked.unwrap_or(action == "check"),
        },
        "press" => {
            let key = a
                .key
                .or(a.text)
                .ok_or("press needs a key (\"key\": \"Enter\")")?;
            vw_ask::chrome::keys::validate(&key, &[])
                .map_err(|reason| format!("not a key: {reason}"))?;
            Action::Press {
                key,
                reference: optional_ref(a.reference, "press")?,
            }
        }
        "scroll" => Action::Scroll {
            reference: optional_ref(a.reference, "scroll")?,
            up: a.direction.as_deref() == Some("up"),
            amount: a.amount.map(|n| n.min(20_000)),
        },
        "upload" => upload(a.reference, a.paths, a.file_ids.is_some())?,
        "wait" | "waitFor" => {
            let reference = optional_ref(a.reference, "wait")?;
            let load = a.load.unwrap_or(false);
            if a.text.is_none() && reference.is_none() && !load {
                return Err("wait needs \"text\", \"ref\" or \"load\": true".to_owned());
            }
            Action::Wait {
                text: a.text.map(|t| t.chars().take(500).collect()),
                reference,
                load,
                timeout_ms: a
                    .timeout_ms
                    .unwrap_or(DEFAULT_WAIT_MS)
                    .clamp(100, MAX_WAIT_MS),
            }
        }
        "" => return Err("browser_act needs an \"action\"".to_owned()),
        other => {
            return Err(format!(
                "unknown action '{}': {ACTIONS}",
                other.chars().take(30).collect::<String>()
            ));
        }
    })
}

fn reload(args: &Value) -> Action {
    if args["bypassCache"] == true {
        Action::ReloadBypassCache
    } else {
        Action::Reload
    }
}

fn coordinate(args: &Value, key: &str) -> Result<u32, String> {
    args[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= 100_000)
        .ok_or_else(|| format!("click {key} must be a nonnegative CSS pixel coordinate"))
}
