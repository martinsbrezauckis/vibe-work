//! Which client sent a command ([`Origin`]: the app's window or the Debug API) and the confirmation dialog's
//! requester for it. Split out of `command.rs` (size limit); `command` re-exports both, so callers use
//! `crate::command::{Origin, requester}` as before.
//! Also [`debug_api_acts`]: who is acting for attribution and the vault rule, which also counts window commands sent
//! while the Debug API drives the window.
//! Depends on: `serde`, `vw_gate`. Called by: `command` (re-export), dispatch and every command handler.
use serde::{Deserialize, Serialize};

/// Which client sent a command. Logged with every refusal so later intent rules can differ per
/// client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// The app's own webview (Tauri bridge).
    Ui,
    /// The loopback Debug API (an external agent or a test).
    DebugApi,
}

/// The dialog's requester for a command's origin, kept as a structured field.
#[must_use]
pub(crate) fn requester(origin: Origin) -> vw_gate::Requester {
    match origin {
        Origin::Ui => vw_gate::Requester::Window,
        Origin::DebugApi => vw_gate::Requester::DebugApi,
    }
}

/// The host's probe: true while a Debug API `ui/*` request is being served by the window (`vw-host` `UiBridge`).
pub(crate) type WindowDrive = Box<dyn Fn() -> bool + Send + Sync>;

/// True when the Debug API is the one acting: its own commands, and window commands sent while it drives the window
/// (a `ui/*` request is being served; the host's probe). Window commands never get the Debug API's authority from
/// this: they keep the window's confirmations. It is read only where the answer attributes an act (who answered a
/// card, the browser's actor, a log label) or restricts it (the vault rule): an injected page script gains nothing
/// by running while a request is served, it is only attributed to the Debug API and restricted like it.
pub(crate) fn debug_api_acts(shared: &crate::core::Shared, origin: Origin) -> bool {
    match origin {
        Origin::DebugApi => true,
        Origin::Ui => shared.window_drive.get().is_some_and(|probe| probe()),
    }
}

impl Origin {
    /// The wire name (`ui`, `debug-api`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::DebugApi => "debug-api",
        }
    }
}
