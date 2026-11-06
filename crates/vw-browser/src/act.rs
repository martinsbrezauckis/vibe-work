//! The `browser_act` request: parsed from the tool's (or the Debug API's) JSON arguments into an
//! [`Action`], every field checked and bounded before anything reaches a page; and the act script's answer.
//!
//! Depends on: serde, `serde_json`, `model` (ref shape). Called by: `driver`, vw-core-api (tool and command
//! arguments), tests.
//! Invariants: a ref must have the ref shape; text to type is at most [`MAX_TEXT`] characters; keys are a fixed list
//! or one printable character; waits are at most [`MAX_WAIT_MS`].

use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::is_ref;

/// Longest text an action types.
pub const MAX_TEXT: usize = 10_000;
/// Longest wait an action asks for (ms), under Vibe's fixed 60 s tool timeout.
pub const MAX_WAIT_MS: u64 = 15_000;
/// Default wait (ms).
pub const DEFAULT_WAIT_MS: u64 = 5_000;
/// Named keys `press` accepts (plus any single printable character).
pub const KEYS: [&str; 15] = [
    "Enter",
    "Tab",
    "Escape",
    "Backspace",
    "Delete",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Space",
    " ",
];

/// One browser action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Load a URL (the navigation policy decides).
    Navigate {
        /// The URL.
        url: String,
    },
    /// History back.
    Back,
    /// History forward.
    Forward,
    /// Reload.
    Reload,
    /// Click an element.
    Click {
        /// Its ref.
        reference: String,
    },
    /// Click a viewport point bound to an observed node.
    ClickAt {
        /// Expected ref; empty resolves against the latest observation.
        reference: String,
        /// CSS x coordinate.
        x: u32,
        /// CSS y coordinate.
        y: u32,
    },
    /// Drag one observed element to another.
    Drag {
        /// Source ref.
        reference: String,
        /// Destination ref.
        to: String,
    },
    /// Replace a field's text (optionally press Enter after).
    Type {
        /// The field's ref.
        reference: String,
        /// The text.
        text: String,
        /// Press Enter afterwards.
        submit: bool,
    },
    /// Choose an option of a select box (by value or visible text).
    Select {
        /// The select's ref.
        reference: String,
        /// The option.
        value: String,
    },
    /// Set a checkbox, radio or switch.
    Check {
        /// Its ref.
        reference: String,
        /// Wanted state.
        checked: bool,
    },
    /// Press a key on an element (or the focused one).
    Press {
        /// The key.
        key: String,
        /// The element (default: the focused one).
        reference: Option<String>,
    },
    /// Scroll an element into view, or the page.
    Scroll {
        /// The element.
        reference: Option<String>,
        /// `down` (default) or `up` when no ref.
        up: bool,
        /// Pixels (default: most of a screen).
        amount: Option<u32>,
    },
    /// Hover an element.
    Hover {
        /// Its ref.
        reference: String,
    },
    /// Attach files to a file field: `paths` as the caller named them, `files` as the host loaded them from the
    /// chat's folders (the driver refuses an upload whose files were not loaded).
    Upload {
        /// The file field (or a label or control a single file field belongs to).
        reference: String,
        /// The files as named in the request (chat-folder relative).
        paths: Vec<String>,
        /// The loaded files.
        files: Vec<UploadFile>,
    },
    /// Wait until text shows, a ref shows, or the page finished loading (bounded).
    Wait {
        /// Text to wait for.
        text: Option<String>,
        /// Ref to wait for.
        reference: Option<String>,
        /// Wait for the page load.
        load: bool,
        /// Limit (ms).
        timeout_ms: u64,
    },
}

/// Most files one upload attaches.
pub const MAX_UPLOAD_FILES: usize = 16;
/// Most bytes one upload attaches, all files together.
pub const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

/// One file for an upload, read by the host from the chat's folders.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadFile {
    /// The file name the page sees.
    pub name: String,
    /// Its media type.
    pub mime: String,
    /// Its bytes.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for UploadFile {
    // Never the bytes: actions are logged.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UploadFile({}, {} bytes)", self.name, self.data.len())
    }
}

impl Action {
    /// Whether this act sends the page input a person's gesture would (a click, a key, typing, a choice, a drag):
    /// the engine then runs it as that gesture, so a popup or `target=_blank` link it opens is not blocked.
    #[must_use]
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::Click { .. }
                | Self::Check { .. }
                | Self::Press { .. }
                | Self::Type { .. }
                | Self::Select { .. }
                | Self::Drag { .. }
        )
    }

    /// The action's name as the tool spells it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "navigate",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Reload => "reload",
            Self::Click { .. } | Self::ClickAt { .. } => "click",
            Self::Type { .. } => "type",
            Self::Drag { .. } => "drag",
            Self::Select { .. } => "select",
            Self::Check { .. } => "check",
            Self::Press { .. } => "press",
            Self::Scroll { .. } => "scroll",
            Self::Hover { .. } => "hover",
            Self::Wait { .. } => "wait",
            Self::Upload { .. } => "upload",
        }
    }

    /// The ref the action targets, if any.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Click { reference }
            | Self::ClickAt { reference, .. }
            | Self::Drag { reference, .. }
            | Self::Type { reference, .. }
            | Self::Select { reference, .. }
            | Self::Check { reference, .. }
            | Self::Hover { reference }
            | Self::Upload { reference, .. } => Some(reference),
            Self::Press { reference, .. }
            | Self::Scroll { reference, .. }
            | Self::Wait { reference, .. } => reference.as_deref(),
            _ => None,
        }
    }

    /// The act script's request (`page` = the page environment accessor); `None` for actions the host does itself.
    #[must_use]
    pub fn page_request(&self, page: &str) -> Option<Value> {
        let r = |action: &str, extra: Value| {
            let mut v = json!({"page": page, "action": action});
            if let (Value::Object(map), Value::Object(more)) = (&mut v, extra) {
                map.extend(more);
            }
            Some(v)
        };
        match self {
            Self::Drag { reference, to } => r("drag", json!({"ref": reference, "to": to})),
            Self::Click { reference } | Self::ClickAt { reference, .. } => {
                r("click", json!({"ref": reference}))
            }
            Self::Hover { reference } => r("hover", json!({"ref": reference})),
            Self::Type {
                reference,
                text,
                submit,
            } => r(
                "type",
                json!({"ref": reference, "text": text, "submit": submit}),
            ),
            Self::Select { reference, value } => {
                r("select", json!({"ref": reference, "value": value}))
            }
            Self::Check { reference, checked } => {
                r("check", json!({"ref": reference, "checked": checked}))
            }
            Self::Press { key, reference } => {
                let mut request = crate::keys::request(key)?;
                request["ref"] = json!(reference);
                r("press", request)
            }
            Self::Upload {
                reference, files, ..
            } => r(
                "upload",
                json!({"ref": reference, "files": files.iter().map(|f| json!({
                    "name": f.name, "mime": f.mime, "size": f.data.len(),
                    "parts": f.data.len().div_ceil(crate::upload::CHUNK),
                })).collect::<Vec<_>>()}),
            ),
            Self::Scroll {
                reference,
                up,
                amount,
            } => r(
                "scroll",
                json!({"ref": reference, "direction": if *up { "up" } else { "down" }, "amount": amount}),
            ),
            _ => None,
        }
    }
}

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

fn optional_ref(value: Option<String>, action: &str) -> Result<Option<String>, String> {
    value.map(|v| reference(Some(v), action)).transpose()
}

/// `upload {ref, paths}`: 1 to [`MAX_UPLOAD_FILES`] chat-folder paths; the host loads the files.
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
    let paths = paths.unwrap_or_default();
    if paths.is_empty() || paths.len() > MAX_UPLOAD_FILES {
        return Err(format!(
            "upload needs \"paths\": 1 to {MAX_UPLOAD_FILES} files in this chat's folders, relative to its folder \
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

/// The actions `parse` knows, for its refusal of another name.
const ACTIONS: &str = "navigate, back, forward, reload, click, type, select, check, press, scroll, hover, drag, wait, upload";

/// Parses `browser_act` arguments.
///
/// # Errors
/// A plain-text reason the agent can act on.
pub fn parse(args: &Value) -> Result<Action, String> {
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
        "reload" => Action::Reload,
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
            value: a
                .value
                .or(a.text)
                .ok_or("select needs the option (\"value\")")?,
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
            if crate::keys::request(&key).is_none() {
                return Err("not a key: press needs a key, optionally preceded by Control, Shift, Alt or Meta (for example Control+a or Shift+Tab)".to_owned());
            }
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

/// The act script's answer.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageAnswer {
    /// `done`, `unmet`, `stale`, `ambiguous`, `secret`, `notActionable`, `noOption`, `error`.
    #[serde(default)]
    pub status: String,
    /// The address of the document that answered (checked against the mount's policy).
    #[serde(default)]
    pub url: String,
    /// The element acted on (role and name: page text).
    #[serde(default)]
    pub target: Option<Target>,
    /// The control that received a key, including when the request used the current focus without a ref.
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    /// Failed actionability checks.
    #[serde(default)]
    pub failed: Vec<String>,
    /// A visibility, movement or hit-test failure during a running entrance animation; the host may poll briefly.
    #[serde(default)]
    pub retryable: bool,
    /// The element covering the target.
    #[serde(default)]
    pub covering: Option<Covering>,
    /// Secret kind (`password`, `otp`, `card`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Options of a select box (when the option was not found).
    #[serde(default)]
    pub options: Vec<String>,
    /// The field's value after typing / the chosen option.
    #[serde(default)]
    pub value: Option<String>,
    /// Checked state after `check`.
    #[serde(default)]
    pub checked: Option<bool>,
    /// What a key press did.
    #[serde(default)]
    pub effect: Option<String>,
    /// Error text.
    #[serde(default)]
    pub message: Option<String>,
    /// Scroll position after a scroll.
    #[serde(default)]
    pub scroll_y: Option<i64>,
    /// After `upload`: the files the field now holds (name and size: what the page sees).
    #[serde(default)]
    pub uploaded: Option<Value>,
    /// After `scroll {ref}`: the element's box in its own document's viewport.
    #[serde(default, rename = "box")]
    pub bounds: Option<crate::frames::RefBox>,
    /// After `scroll {ref}`: that document's viewport (CSS pixels).
    #[serde(default)]
    pub viewport: Option<(u32, u32)>,
    /// Live geometry prepared without events; only a host-authorized native action consumes it.
    #[serde(default)]
    pub native_point: Option<vw_preview::NativeClickPoint>,
}

/// An element as the act script describes it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, serde::Serialize)]
pub struct Target {
    /// Role.
    #[serde(default)]
    pub role: String,
    /// Name.
    #[serde(default)]
    pub name: String,
}

/// The element covering a target.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, serde::Serialize)]
pub struct Covering {
    /// Its ref.
    #[serde(default, rename = "ref")]
    pub id: String,
    /// Role.
    #[serde(default)]
    pub role: String,
    /// Name.
    #[serde(default)]
    pub name: String,
    /// Tag.
    #[serde(default)]
    pub tag: String,
}

fn coordinate(args: &Value, key: &str) -> Result<u32, String> {
    args[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= 100_000)
        .ok_or_else(|| format!("click {key} must be a nonnegative CSS pixel coordinate"))
}
