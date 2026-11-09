//! Typed screenshot refusals travel beside unchanged agent guidance, from visibility checks to the Capture menu.
//! Depends on serde; preview service produces them, browser/core errors preserve them for UI clients.
use serde::{Deserialize, Serialize};

/// Why the current surface cannot be captured, independent of the agent-facing sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CaptureBlock {
    /// The main window is minimized or closed to the tray/menu bar.
    WindowHidden,
    /// The window shows another page.
    AnotherPage,
    /// The window shows New chat.
    NoChatSelected,
    /// The window shows another session.
    AnotherSession,
    /// The Browser panel is not open.
    BrowserPanelClosed,
    /// The engine is paused to save memory.
    BrowserPaused,
    /// A web tab is selected instead of App preview.
    AppPreviewNotSelected,
    /// Another tab is selected instead of the requested web tab.
    WebTabNotSelected,
    /// App preview is not mounted in the panel.
    AppPreviewNotShown,
    /// The web tab is not mounted in the panel.
    WebTabNotShown,
    /// The preview's server is stopped.
    ServerStopped,
    /// A native Browser overlay has no shown page.
    BrowserPageNotShown,
}

/// A stable reason and the complete diagnosis/instruction for the agent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{text}")]
pub struct CaptureRefusal {
    /// Stable reason for the window's short label.
    pub kind: CaptureBlock,
    /// The unchanged agent-facing reason and next action.
    pub text: String,
}

impl CaptureBlock {
    pub(crate) fn because(self, text: impl Into<String>) -> CaptureRefusal {
        CaptureRefusal {
            kind: self,
            text: text.into(),
        }
    }
}

impl std::ops::Deref for CaptureRefusal {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
