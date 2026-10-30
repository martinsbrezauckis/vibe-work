//! The host of the Vibe Work app: everything the Tauri crate would otherwise have to contain,
//! kept here so it is testable without a window. Logic lives in `crates/`;
//! `app/` wires the Tauri integration.
//!
//! Responsibilities:
//! - [`Host`]: starts the core ([`vw_core_api::Core`]) and the Debug API
//!   ([`vw_debug_api::DebugApi`]) from a [`HostConfig`], answers the UI's commands with
//!   `origin = ui`, forwards core events to whatever the UI listens on ([`Host::subscribe`]), and
//!   shuts everything down.
//! - [`HostHandle`]: lets the window open at once while the host starts in the background.
//! - [`UiBridge`]: the broker behind the Debug API's `ui/*` routes. A route handler cannot reach
//!   the webview itself, so it parks a request in the bridge; the UI picks it up (Tauri command or
//!   the `ui/ui-client/poll` route), runs it against its own state and answers.
//! - [`SharedConfirmer`], [`HostEvent`], notifications and folder reveal: the native-dialog seam, the
//!   events the host itself publishes, and the OS actions the UI may request.
//! - Documents ([`Host::add_documents`]): paths from the native picker or an OS drop go
//!   from the Tauri side to the core; the webview gets ids and facts only.
//! - Terminal output stream ([`Host::terminal_attach`]): raw bytes with flow control for
//!   the window's terminal view; terminal commands themselves go through [`Host::execute`].
//! - [`launch`]: the development launch config (test `VIBE_HOME`, never the user's `~/.vibe`).
//! - [`product`]: the user-build launch: Vibe installed at setup (or not yet), own
//!   `VIBE_HOME`, key service, built-in plugin, CLI session sharing.
//! - [`updates`]: whether this build may check for updates (fail closed) and the last update step's status.
//! - [`update_flow`]: notify, download, then ask: the update state machine and the consent that gates
//!   every install (`update_store`: the verified package on disk, `update_verify`: minisign check, `update_view`:
//!   the view and the traits the app implements; `update_leftovers`: the Windows installer copies an update leaves
//!   in the temp folder, removed at the next start; `update_os`: the oldest system a new version runs on).
//! - [`navigation`]: the policy that keeps the main window on the app's own origin.
//! - [`media`]: the one web permission the main window grants (the microphone, for dictation).
//! - [`settings`]: the few persisted switches the host needs before the UI exists (Debug API).
//! - [`agent_tools`]: turns the session tools on for a run: the `vw-agent-tools` binary, the
//!   kill guard in our `VIBE_HOME/hooks.toml`, the built-in plugin root.
//! - [`title_model`]: the model Vibe writes session titles with, set in our own `VIBE_HOME/config.toml`.
//!
//! Depends on: vw-core-api, vw-debug-api, vw-gate, vw-supervisor. Called by: `app` (Tauri
//! wiring), the private test host.
//! Invariants:
//! - Starting never panics and never aborts the app: a failed or missing core is reported as a
//!   [`Startup`] state and every command then fails with `serverUnavailable` and that reason.
//! - Window commands are executed with [`Origin::Ui`](vw_core_api::Origin). Only a window command that matches a
//!   grant of a pending Debug API `ui/*` request runs with `Origin::DebugApi` (that origin acts as the user without
//!   dialogs), once, while the Debug API is on; grants are built in Rust from the request alone ([`ui_grants`]).
//!   Nothing the window sends can claim the Debug API origin.

pub mod agent_tools;
mod bridge;
pub mod cli_sharing;
pub mod commit_signature;
mod confirm;
pub mod default_model;
mod documents;
mod events;
pub mod exit;
mod export;
mod extension_paths;
mod files_routes;
mod forward;
pub mod git_bash;
pub mod hand_over;
mod handle;
mod host;
pub mod instance;
pub mod launch;
mod launch_version;
pub mod media;
pub mod navigation;
mod notify;
pub mod product;
pub mod send_to;
pub mod settings;
mod terminal;
pub mod title_model;
pub mod trust_store;
pub mod ui_grants;
pub mod update_appimage;
mod update_consent;
mod update_download;
pub mod update_flow;
pub mod update_leftovers;
pub mod update_os;
mod update_store;
pub mod update_verify;
mod update_view;
pub mod updates;

pub use bridge::{UiBridge, UiOutcome, UiReply, UiReplyError, UiRequest};
pub use confirm::{
    AGENT_LINE, ALLOW_ONCE, CLOSE, CONTINUE, DEBUG_API_LINE, DIALOG_PARAMS_MAX_CHARS, DIALOG_TITLE,
    DONT_ALLOW, MAX_PAGES, NEXT_PART, PAGE_CHARS, SHORT_HEAD, SHORT_TAIL, SHOW_FULL,
    SharedConfirmer, ShortOutcome, ShortStep, dialog_answer, dialog_buttons, dialog_heading,
    dialog_text, full_pages, shortened_answer, shortened_buttons, shortened_dialog_text,
    system_dialog_order,
};
pub use documents::DocumentsPicked;
pub use events::HostEvent;
pub use forward::Forwarder;
pub use handle::HostHandle;
pub use host::{DEFAULT_UI_REPLY_TIMEOUT, Host, HostConfig, Startup};
pub use notify::{BURST, NotifyRefusal, NotifyRequest};
pub use terminal::TerminalAttached;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str =
    "app host: core and Debug API lifecycle, event forwarding, UI-control broker";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
