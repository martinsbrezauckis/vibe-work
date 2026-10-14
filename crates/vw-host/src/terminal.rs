//! The app window's terminal output stream: attach a view to a terminal, acknowledge
//! what it rendered, detach. Commands (open, write, ...) go through `Host::execute` like every other
//! command; only the byte stream needs its own path, because it is raw bytes with flow control.
//!
//! Depends on: vw-core-api (`Core::terminal_attach` and friends). Called by: `app` (the Tauri
//! commands `terminal_attach`, `terminal_ack`, `terminal_detach`), the private test host.
//! Invariants:
//! - Only the app's own window reaches these (Tauri commands granted to `main`); the Debug API reads
//!   output through the `terminal` command (`read`), never through a viewer, so it cannot stall a
//!   program by not acknowledging.
//! - Also registers the `ui/terminal` route description (the UI implements the handler).

use serde::Serialize;
use vw_core_api::CoreError;

use crate::host::Host;

/// The `ui/*` routes this block registers: `(name, description)`.
pub(crate) const TERMINAL_ROUTES: &[(&str, &str)] = &[(
    "terminal",
    "The Terminal tab of the shown session. Body: {action: state|selection|selectAll|focus|open|select, terminalId?}: state = its terminal tabs, the active one, whether a terminal has keyboard focus, font size and renderer; selection = the selected text of the active terminal (read only, for \"Point Vibe at this\"); selectAll selects the whole buffer; focus focuses the active terminal; open opens a new terminal tab (same as the + button); select makes terminalId the active tab.",
)];

/// What the window learns when it attaches to a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAttached {
    /// The viewer id to pass to ack and detach.
    pub viewer: u64,
    /// Offset of the first replayed byte.
    pub start: u64,
    /// Offset just past the replay.
    pub end: u64,
}

impl Host {
    /// Attaches the window's terminal view: the scrollback is replayed into `sink`, live output
    /// follows; `sink` returns `false` when the window is gone (the terminal then keeps buffering).
    ///
    /// # Errors
    /// `notFound` for an unknown terminal; `serverUnavailable` without a core.
    pub fn terminal_attach<S>(
        &self,
        terminal_id: &str,
        sink: S,
    ) -> Result<TerminalAttached, CoreError>
    where
        S: FnMut(&[u8]) -> bool + Send + 'static,
    {
        let core = self.core_or_unavailable()?;
        let info = core.terminal_attach(terminal_id, Box::new(sink))?;
        Ok(TerminalAttached {
            viewer: info.viewer,
            start: info.start,
            end: info.end,
        })
    }

    /// The window rendered `bytes` more bytes of `terminal_id` (flow control).
    ///
    /// # Errors
    /// `serverUnavailable` without a core.
    pub fn terminal_ack(
        &self,
        terminal_id: &str,
        viewer: u64,
        bytes: u64,
    ) -> Result<(), CoreError> {
        self.core_or_unavailable()?
            .terminal_ack(terminal_id, viewer, bytes);
        Ok(())
    }

    /// The window stopped showing `terminal_id` (it keeps running and buffering).
    ///
    /// # Errors
    /// `serverUnavailable` without a core.
    pub fn terminal_detach(&self, terminal_id: &str, viewer: u64) -> Result<(), CoreError> {
        self.core_or_unavailable()?
            .terminal_detach(terminal_id, viewer);
        Ok(())
    }
}
