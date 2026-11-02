//! VIBE WORK PATCH: a gate before the macOS file open panel. The embedding app refuses a panel that a page opened
//! during the app's own automated act, so it never appears on the person's screen; the person's own clicks keep
//! the panel. A refused panel answers "no file", as Cancel does.

use std::sync::OnceLock;

/// Called with the web view's address; `true` refuses the panel.
pub type FilePanelGate = Box<dyn Fn(usize) -> bool + Send + Sync>;

static GATE: OnceLock<FilePanelGate> = OnceLock::new();

/// Installs the gate (once per process; later calls are ignored and answer `false`).
pub fn set_file_panel_gate(gate: FilePanelGate) -> bool {
  GATE.set(gate).is_ok()
}

#[cfg(target_os = "macos")]
pub(crate) fn refused(webview: usize) -> bool {
  GATE.get().is_some_and(|gate| gate(webview))
}
