//! `PreviewError`: why a preview operation failed, shared by the service, its tabs and the agent browser.
//! Depends on: `engine` (engine faults), `policy` (URL refusals). Re-exported by `service` and the crate root.

use crate::engine::EngineError;

/// Why a preview operation failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    /// Nothing is open in this session's preview.
    #[error("this session has no preview open (open one first)")]
    NothingOpen,
    /// A hidden native webview can return a blank PNG even though its page is loaded.
    #[error("Not run: {0}")]
    ScreenshotHidden(String),
    /// The URL was refused.
    #[error(transparent)]
    Refused(#[from] crate::policy::UrlRefusal),
    /// Invalid layout input.
    #[error("{0}")]
    Layout(String),
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The screenshot could not be shaped.
    #[error("{0}")]
    Image(String),
    /// The external browser cap has no free engine.
    #[error("Not run: the browser is busy with another session; try again when it is free")]
    BrowserBusy,
    /// The pool setting is invalid.
    #[error("browser pool setting: {0}")]
    PoolSetting(String),
    /// The pool setting could not be saved.
    #[error("browser pool storage: {0}")]
    PoolStorage(String),
    /// A page key is unknown, belongs to another session or violates tab invariants.
    #[error("browser tab: {0}")]
    InvalidTab(String),
    /// The page is no longer the selected, shown one (a tab switch overtook an overlay request).
    #[error("browser tab: page is no longer selected and visible")]
    NotInView,
}
