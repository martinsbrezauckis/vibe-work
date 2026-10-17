//! The agent browser part of the command API: the requests the window and the Debug API send inside
//! `Command::Browser`, and what comes back. The loop and its rules are in vw-browser; the handler is `browser`.
//!
//! Depends on: vw-browser (state, annotation types), serde. Called by: `command`, `browser`, the UI through the
//! generated TypeScript, vw-debug-api.
//! Invariants: wire form `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused; observations and
//! receipts are JSON whose page-derived part sits under `untrustedPageContent`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vw_browser::BrowserState;
use vw_browser::annotate::{Annotation, RecheckItem};

/// One request about a session's browser. Vault choice and save requests are window-only; other Debug API actions
/// are labelled `debugApi` in the action log and are not affected by Stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum BrowserRequest {
    /// The pinned app preview and this session's web tabs.
    Tabs {
        /// The loaded session.
        session_id: String,
    },
    /// Opens a web tab; an optional URL is checked with the user's navigation rights.
    OpenTab {
        /// The loaded session.
        session_id: String,
        /// An optional initial address.
        #[serde(default)]
        url: Option<String>,
    },
    /// Selects a tab in the Browser panel.
    SwitchTab {
        /// The loaded session.
        session_id: String,
        /// App preview session id or owned web page key.
        tab_id: String,
    },
    /// Closes a web tab. The app preview cannot be closed here.
    CloseTab {
        /// The loaded session.
        session_id: String,
        /// Owned web page key.
        tab_id: String,
    },
    /// Sets the complete web-tab order; the pinned app preview stays first.
    ReorderTabs {
        /// The loaded session.
        session_id: String,
        /// Every web page key once, in the requested order.
        tab_ids: Vec<String>,
    },
    /// Reviews the current page and owner before granting the agent use of a user tab.
    ReviewHandoff {
        /// The loaded session.
        session_id: String,
        /// User-controlled web page key.
        tab_id: String,
    },
    /// Grants this exact reviewed user tab to the agent, without granting vault rights.
    HandoffTab {
        /// The loaded session.
        session_id: String,
        /// Reviewed user-controlled web page key.
        tab_id: String,
        /// Identity of the exact page, mount and owner just reviewed.
        fingerprint: String,
        /// Explicit acceptance of that review.
        confirmed: bool,
    },
    /// Returns a delegated tab to user control and cancels pending agent acts in it.
    TakeBackTab {
        /// The loaded session.
        session_id: String,
        /// Delegated web page key.
        tab_id: String,
    },
    /// The user's browser tab-close permission (off by default).
    Permissions {},
    /// Saves the user-tab-close permission.
    SetPermissions {
        /// The complete replacement contract.
        permissions: vw_browser::delegation::BrowserPermissions,
    },
    /// The browser engine pool's memory allowance, setting and session occupancy.
    EnginePool {},
    /// Sets the parallel browser allowance; memory can reduce a fixed value.
    SetEnginePool {
        /// `auto` (default), or a number from `1` through `4`.
        configured_parallel_sessions: String,
    },
    /// Cookie-presence hints for the shared browser's identity providers.
    Accounts {},
    /// Opens a provider's fixed sign-in page for direct user input only.
    SignIn {
        /// The provider to sign into.
        provider: vw_preview::accounts::Provider,
    },
    /// Clears this provider's cookies from the shared browser profile.
    SignOut {
        /// The provider to sign out of.
        provider: vw_preview::accounts::Provider,
    },
    /// Places or hides the user-only sign-in browser in Settings.
    AccountsLayout {
        /// The visible page rectangle; absent hides it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        rect: Option<vw_preview::Rect>,
    },
    /// Closes the user-only sign-in page, retaining its provider sessions.
    CloseSignIn {},
    /// The page as the agent reads it (`browser_read`).
    Read {
        /// The session.
        session_id: String,
        /// An owned page in this session; absent uses the user's selected page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// Refs to list (default 32, at most 200).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        max_refs: Option<u32>,
        /// A whole text page (0-based, 2,500 characters each); absent: an 800-character preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        text_page: Option<u32>,
        /// The answer's size budget in bytes (default 3,000; 1,500 to 64,000).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        max_bytes: Option<u32>,
        /// A focused read: only the refs and text lines containing this text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        find: Option<String>,
        /// Also a screenshot.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        screenshot: Option<bool>,
    },
    /// One action (`browser_act` arguments: `{action, ref?, url?, text?, ...}`), answered with a receipt.
    Act {
        /// The session.
        session_id: String,
        /// An owned page in this session; absent uses the user's selected page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// The action's arguments, exactly as the agent tool takes them.
        args: Value,
    },
    /// The Browser tab's agent strip and action log.
    State {
        /// The session.
        session_id: String,
    },
    /// The user's Stop: cancels the agent's action in flight, refuses further agent actions until Resume or the
    /// session's next turn.
    Stop {
        /// The session.
        session_id: String,
    },
    /// Lets the agent use the browser again.
    Resume {
        /// The session.
        session_id: String,
    },
    /// "Annotate this page": the user clicks an element (`element`, default) or draws a box (`box`) in the
    /// Browser tab; answers what and where with a screenshot crop (untrusted page data). With `target` the element
    /// or area is named instead and the answer is immediate; a Debug API request must name it (`ref`, `selector`, or
    /// `x` and `y` for an element; `rect` for a box): there is no person to point.
    Annotate {
        /// The session.
        session_id: String,
        /// `element` or `box`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        mode: Option<String>,
        /// The element or area to annotate without a pointer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        target: Option<crate::preview_view::PickTarget>,
    },
    /// Obsolete capture command, refused; saving is a user action in the browser panel.
    CaptureSecret {
        /// The session.
        session_id: String,
        /// A ref from the latest observation.
        #[serde(rename = "ref")]
        #[cfg_attr(feature = "ts", ts(rename = "ref"))]
        reference: String,
        /// The secret's name.
        name: String,
        /// `"mcp"` for a key meant for an MCP server (a suggestion only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
    },
    /// The user chooses selected text or starts an element picker; the answer carries only metadata.
    ChooseSecret {
        /// The browser session.
        session_id: String,
        /// Selected page by default; may name a web tab or the pinned App preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// `selection` for current text, `element` to wait for a click, or `contextSelection` for a page menu choice.
        mode: String,
    },
    /// The user names and confirms a prior choice held in native memory.
    SaveSecret {
        /// The browser session.
        session_id: String,
        /// The page that issued the one-use choice.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// One-use handle returned by `chooseSecret`.
        handle: String,
        /// Vault name entered by the user.
        name: String,
    },
    /// Cancels the name prompt and discards only its one-use choice.
    CancelSecret {
        /// The browser session.
        session_id: String,
        /// The page that issued the one-use choice.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// The handle of the dismissed prompt.
        handle: String,
    },
    /// Capture from the page (`args` exactly as the agent's `browser_capture`: `{what, formats?}`, plus
    /// `shotMethod` `native` / `stitch` to force one full-page method): files in the session folder under
    /// `capture/<slug>-<time>/`, answered with the folder, the files and a summary.
    Capture {
        /// The session.
        session_id: String,
        /// Selected page by default; may name a web tab or the pinned App preview.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tab_id: Option<String>,
        /// `{what: design|structure|media|page|bundle|before|after, formats?: [markdown|text|html|png], shotMethod?}`.
        args: Value,
    },
    /// Cancels a running annotation (removes the overlay).
    CancelAnnotate {
        /// The session.
        session_id: String,
    },
    /// After the agent worked on sent annotations: reload the page (optional) and say per annotation whether its
    /// element `changed`, is `unchanged`, or is `missing`.
    CheckAnnotations {
        /// The session.
        session_id: String,
        /// The annotations (at most 50).
        items: Vec<RecheckItem>,
        /// Reload first.
        #[serde(default)]
        reload: bool,
    },
}

/// A PNG answered with an observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserShot {
    /// Base64 PNG (at most 1280 px wide, 1 MiB).
    pub png_base64: String,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Safe metadata for a user-selected browser secret. The value remains in native memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SecretChoiceView {
    /// One-use handle.
    pub handle: String,
    /// Suggested name based on the page host.
    pub suggested_name: String,
    /// Page origin.
    pub page: String,
    /// Generic description of the chosen element.
    pub element: String,
    /// Character count.
    pub value_length: usize,
}

/// What a browser request answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BrowserResult {
    /// The pinned app preview and web tabs after a tab command.
    Tabs {
        /// Pinned app preview followed by the ordered web tabs.
        tabs: Vec<vw_preview::PageTab>,
    },
    /// An identity-bound page review for a user-to-agent handoff.
    HandoffReview {
        /// Identity-bound current-page metadata and fingerprint.
        review: Value,
    },
    /// The stored browser delegation contract.
    Permissions {
        /// User-selected permissions.
        permissions: vw_browser::delegation::BrowserPermissions,
    },
    /// The current engine allowance and the sessions using or waiting for it.
    EnginePool {
        /// Settings, memory detection and occupancy.
        pool: vw_preview::EnginePoolSnapshot,
    },
    /// Provider session-cookie presence only; remote session validity is not asserted.
    Accounts {
        /// One presence hint per provider.
        accounts: Vec<vw_preview::accounts::ProviderAccount>,
    },
    /// `read`.
    Observation {
        /// The observation (the same JSON `browser_read` answers the agent).
        observation: Value,
        /// The screenshot, when asked.
        screenshot: Option<BrowserShot>,
    },
    /// `act`.
    Receipt {
        /// The receipt (the same JSON `browser_act` answers the agent).
        receipt: Value,
    },
    /// `state`, `stop`, `resume`.
    State {
        /// The strip and the log.
        state: BrowserState,
    },
    /// `annotate`.
    Annotation {
        /// What the user pointed at.
        annotation: Annotation,
    },
    /// `checkAnnotations`: one state per item.
    AnnotationStates {
        /// `changed`, `unchanged` or `missing`.
        states: Vec<String>,
    },
    /// `captureSecret`: what was stored (never the value).
    SecretStored {
        /// `{stored, id, length, replaced}`.
        stored: Value,
    },
    /// `chooseSecret`: a handle and metadata, never the chosen value.
    SecretChoice {
        /// Safe metadata for the name prompt.
        choice: SecretChoiceView,
    },
    /// `capture`.
    Capture {
        /// The capture's answer (the same JSON `browser_capture` answers the agent).
        capture: Value,
    },
    /// `cancelAnnotate` or `cancelSecret`.
    Done {},
}
