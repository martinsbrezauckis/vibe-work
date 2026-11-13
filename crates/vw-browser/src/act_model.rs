//! Browser action and page-answer data shapes. Depends on browser request types and serde; called by act parsing and drivers.
use serde::Deserialize;
use serde_json::{Value, json};

/// One browser action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Bounded action screenshots, or an explicit unavailable video result.
    Recording(crate::recording::Request),
    /// Read native tab audio, or set mute and read the actual native state.
    TabAudio(Option<bool>),
    /// Read or control page video/audio with a live state receipt.
    Media(crate::playback::Request),
    /// Read native requests and their headers.
    Network(crate::network::Request),
    /// Run an explicitly authorized expression in the page world.
    PageScript(crate::page_scripts::Request),
    /// Set a responsive CSS viewport, or restore the panel size.
    Viewport(crate::viewport::Request),
    /// Inspect one element's markup, attribute and computed styles.
    Inspect(crate::element_inspection::Request),
    /// Read or clear current-document diagnostic records.
    Diagnostics(crate::diagnostics::Request),
    /// Resolve a point against the latest observation without sending input.
    ResolvePoint {
        /// CSS viewport horizontal coordinate.
        x: u32,
        /// CSS viewport vertical coordinate.
        y: u32,
    },
    /// Checked focus, blur or insertion at the current selection.
    Control(crate::control::Request),
    /// Bounded document, address or element-state wait.
    WaitCondition(crate::wait_condition::Request),
    /// Scroll a particular pane or document and inspect its offsets.
    ScrollMotion(crate::scroll_motion::Request),
    /// Read or answer a held native page dialog.
    Dialog(crate::dialogs::Request),
    /// A native click variant wrapping one ordinary checked ref or coordinate target.
    Pointer {
        /// The ref or coordinate click.
        base: Box<Action>,
        /// Button, count and modifiers.
        options: vw_preview::ClickOptions,
    },
    /// Current-site local or session storage.
    Storage(crate::site_storage::Request),
    /// Current-site native cookie store.
    Cookies(vw_preview::CookieRequest),
    /// Chat-scoped download management.
    Downloads(crate::downloads::Request),
    /// Capture a viewport, one visible element, or a rectangle.
    Screenshot(crate::screenshot_area::Area),
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
    /// Reload the page while bypassing the HTTP cache.
    ReloadBypassCache,
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
        /// The requested options; an empty list clears a multiple select.
        values: Vec<String>,
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
            Self::Control(_)
                | Self::Click { .. }
                | Self::Pointer { .. }
                | Self::Check { .. }
                | Self::Press { .. }
                | Self::Type { .. }
                | Self::Select { .. }
                | Self::Drag { .. }
        )
    }

    /// The action's name as the tool spells it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Recording(request) => request.name(),
            Self::TabAudio(None) => "tabs.audio",
            Self::TabAudio(Some(_)) => "tabs.mute",
            Self::Media(request) => request.name(),
            Self::Network(request) => request.name(),
            Self::PageScript(request) => request.name(),
            Self::Viewport(_) => "tabs.emulate",
            Self::Inspect(_) => "page.inspect",
            Self::Diagnostics(request) => request.name(),
            Self::ResolvePoint { .. } => "resolve_point",
            Self::ScrollMotion(request) => request.name(),
            Self::Control(request) => request.name(),
            Self::Pointer { options, .. } => crate::pointer::name(*options),
            Self::Dialog(request) => request.name(),
            Self::Storage(request) => request.name(),
            Self::Cookies(request) => request.name(),
            Self::Downloads(request) => request.name(),
            Self::Screenshot(_) => "capture.screenshot",
            Self::Navigate { .. } => "navigate",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Reload | Self::ReloadBypassCache => "reload",
            Self::Click { .. } | Self::ClickAt { .. } => "click",
            Self::Type { .. } => "type",
            Self::Drag { .. } => "drag",
            Self::Select { .. } => "select",
            Self::Check { .. } => "check",
            Self::Press { .. } => "press",
            Self::Scroll { .. } => "scroll",
            Self::Hover { .. } => "hover",
            Self::WaitCondition(_) | Self::Wait { .. } => "wait",
            Self::Upload { .. } => "upload",
        }
    }

    /// The ref the action targets, if any.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::ScrollMotion(request) => request.reference(),
            Self::WaitCondition(request) => request.reference(),
            Self::Control(request) => Some(request.reference()),
            Self::Pointer { base, .. } => base.reference(),
            Self::Screenshot(area) => area.reference(),
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
        if let Self::Pointer { base, .. } = self {
            return base.page_request(page);
        }
        let r = |action: &str, extra: Value| {
            let mut v = json!({"page": page, "action": action});
            if let (Value::Object(map), Value::Object(more)) = (&mut v, extra) {
                map.extend(more);
            }
            Some(v)
        };
        match self {
            Self::ScrollMotion(request) => r(request.name(), request.args()),
            Self::Control(request) => r(request.name(), request.args()),
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
            Self::Select { reference, values } => {
                r("select", json!({"ref": reference, "values": values}))
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
    /// Secret kind (`password`, `card`).
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
    /// Observed pane/document offsets and edges.
    pub scroll: Option<Value>,
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
