//! The preview part of the command API: the requests both clients send inside `Command::Preview`, and
//! what comes back. The model and its rules are in vw-preview; the handler is `previews`.
//!
//! Depends on: vw-preview (views), serde. Called by: `command`, `previews`, the UI through the generated
//! TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A preview URL never comes from the result of another session; targets list this session's ports only.
//! - Process views carry the executable name and pid only (rule: never arguments).

use serde::{Deserialize, Serialize};
use vw_preview::{ConsoleEntry, DeviceWidth, PreviewView, Rect, VisibilityReport};

/// Native Browser page handoff for menus that visually cover the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum BrowserOverlayAction {
    Prepare,
    Show,
    Close,
}

/// One request about a session's preview. From the Debug API origin, `open`, `back`, `forward`, `reload`,
/// `setWidth`, `close` and `stopOwner` need the native confirmation (they change what runs or loads); reads and
/// `layout` (where the window shows it) run directly. From the app's own window everything runs directly
/// (a trusted surface).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreviewRequest {
    /// The session's preview, the servers it may open, its leased ports and busy-port explanations.
    List {
        /// The session.
        session_id: String,
    },
    /// Opens a URL (or `http://127.0.0.1:<port><path>`) of this session in its preview.
    Open {
        /// The session.
        session_id: String,
        /// A loopback `http` URL of this session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        url: Option<String>,
        /// Or a port of this session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        port: Option<u16>,
        /// Path with `port` (default `/`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        path: Option<String>,
    },
    /// History back.
    Back {
        /// The session.
        session_id: String,
    },
    /// History forward.
    Forward {
        /// The session.
        session_id: String,
    },
    /// Reload.
    Reload {
        /// The session.
        session_id: String,
    },
    /// Device width (390 / 768 / 1280 / fit).
    SetWidth {
        /// The session.
        session_id: String,
        /// A page owned by this session. Omitted targets the pinned App preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// The width.
        width: DeviceWidth,
    },
    /// Selected Preview panel and its native rectangle; a selected panel can be parked under a modal.
    Layout {
        /// The selected session (absent: no Preview panel selected).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// Selected Browser page. Omitted selects the pinned App preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// The page area in the main window's CSS pixels (absent: park the native view).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        rect: Option<Rect>,
    },
    /// The main window's current Browser visibility, reported when window or selection state changes.
    Visibility {
        /// Current UI facts; the service picks the first screenshot refusal reason for each target.
        report: VisibilityReport,
    },
    /// Native Tauri window visibility, sent on minimize and show events.
    NativeWindowVisibility {
        /// False when minimized or hidden.
        visible: bool,
    },
    /// Prepares, parks or restores the selected native page for HTML chrome.
    BrowserOverlay {
        /// Owning session.
        session_id: String,
        /// Selected Browser tab; absent targets the pinned App preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// Prepare returns exact child bounds; show parks and close restores the child.
        action: BrowserOverlayAction,
    },
    /// Closes the session's preview.
    Close {
        /// The session.
        session_id: String,
    },
    /// A PNG of the preview (base64, at most 1280 px wide and 1 MiB).
    Screenshot {
        /// The session.
        session_id: String,
    },
    /// Recent console errors and warnings of the page.
    Console {
        /// The session.
        session_id: String,
        /// At most this many (default and maximum 50).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
    },
    /// Who holds a port and what this session can do about it.
    Explain {
        /// The asking session.
        session_id: String,
        /// The port.
        port: u16,
    },
    /// Stops the holder of a busy port when it is one of our sessions' processes (the user saw the
    /// explanation: name, pid, owner). Another app's process is never stopped.
    StopOwner {
        /// The asking session.
        session_id: String,
        /// The busy port.
        port: u16,
    },
    /// "Point Vibe at this": lets the user point at an element of the session's preview (the
    /// selection, else the next click; Escape cancels) and answers what it is (untrusted page data). With `target`
    /// the element is named instead and the answer is immediate; a Debug API request must name it (`selector`, or
    /// `x` and `y`): there is no person to click.
    Pick {
        /// The session.
        session_id: String,
        /// The element to pick without a click.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        target: Option<PickTarget>,
    },
    /// Cancels a running pick in the session's preview.
    CancelPick {
        /// The session.
        session_id: String,
    },
}

/// What a pick points at when nobody clicks: a Debug API request names it (it has no pointer). Exactly one way:
/// `selector` (the first match), `x` and `y` (the element at that point of the page's viewport, CSS pixels), for the
/// agent browser's annotation also `ref` (an element ref of the last read) or, in box mode, `rect`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickTarget {
    /// A CSS selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub selector: Option<String>,
    /// An element ref from the agent browser's last read (annotation only).
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, rename = "ref"))]
    pub reference: Option<String>,
    /// Left of the point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub x: Option<f64>,
    /// Top of the point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub y: Option<f64>,
    /// The area of a box annotation (viewport CSS pixels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rect: Option<Rect>,
}

/// A server of the session the preview may open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PreviewTarget {
    /// The port.
    pub port: u16,
    /// Its loopback URL.
    pub url: String,
    /// The listening process, when one listens (pid).
    pub pid: Option<u32>,
    /// Its executable name.
    pub process_name: Option<String>,
    /// True when something listens on it now (false: a leased port nothing uses yet).
    pub listening: bool,
    /// True when the session's ownership is a fact (started by Vibe Work, re-attached, or a leased port); false
    /// when it is only inferred from the folder. Only proven ports open for the agent and the Debug API.
    pub proven: bool,
}

/// One thing that can be done about a busy port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BusyOption {
    /// Use this free port of the session's lease instead.
    UseNextLeasedPort {
        /// The port.
        port: u16,
    },
    /// Stop the holder (one of our sessions' processes).
    StopOwner {
        /// Pid of the holder.
        pid: u32,
    },
    /// Open what already runs there.
    OpenRunning {
        /// Its URL.
        url: String,
    },
    /// Nothing holds it now: try again.
    Retry {},
}

/// Who holds a busy port and what can be done (`explain`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BusyPortView {
    /// The port.
    pub port: Option<u16>,
    /// "this session", "another session ('Shop')", "another app (node)", or "unknown (not visible to Vibe Work)".
    pub holder: String,
    /// The holder's pid, when known.
    pub holder_pid: Option<u32>,
    /// The output line that reported it (untrusted process output, at most 300 characters), when it came
    /// from a process's output.
    pub line: Option<String>,
    /// What can be done, best first.
    pub options: Vec<BusyOption>,
}

/// What a preview request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PreviewResult {
    /// `list`.
    Previews {
        /// The session's preview (`None` before anything was opened).
        preview: Option<PreviewView>,
        /// Ports the preview may open: the session's listeners first, then free leased ports.
        targets: Vec<PreviewTarget>,
        /// The session's leased ports.
        leased_ports: Vec<u16>,
        /// Ports this session's servers could not use ("address in use"), explained.
        busy: Vec<BusyPortView>,
    },
    /// `open`, `back`, `forward`, `reload`, `setWidth`, `close`.
    Preview {
        /// The preview after the request.
        preview: Option<PreviewView>,
    },
    /// `screenshot`.
    Screenshot {
        /// PNG, standard base64.
        png_base64: String,
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// `browserOverlay`: exact selected child bounds during prepare.
    Overlay {
        /// Retained for typed client compatibility; parking does not return copied pixels.
        snapshot: Option<String>,
        /// `None` for show and close.
        bounds: Option<Rect>,
    },
    /// `console`.
    Console {
        /// Entries, oldest first (untrusted page text).
        entries: Vec<ConsoleEntry>,
        /// Entries dropped from the host ring so far.
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        dropped: u64,
    },
    /// `explain`.
    Explanation {
        /// The explanation.
        explanation: BusyPortView,
    },
    /// `pick`.
    Picked {
        /// What the user pointed at (untrusted page data, length-capped).
        picked: vw_pages::Picked,
    },
    /// `layout`, `stopOwner`, `cancelPick`.
    Done {},
}
