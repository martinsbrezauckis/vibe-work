//! The preview engine on Tauri: child webviews inside the main window, one per session,
//! with a shared profile for public Web pages and per-session Local/Lan website data, no IPC capability, and every
//! navigation, popup, permission and download decided
//! by the preview model (`vw-preview`).
//!
//! Depends on: tauri (feature `unstable`: `Window::add_child`), vw-preview, vw-platform (`webview` feature), and
//! on Linux gtk/webkit2gtk/cairo. Called by: the app (`startup.rs` builds a [`TauriEngine`] for the core).
//! Invariants:
//! - No preview webview is ever granted a capability; the app's capability file names the `main` webview only.
//! - Platform code that needs `unsafe` (`WebView2` COM, `WKWebView`) lives in vw-platform behind a safe API.

mod engine;
#[cfg(target_os = "linux")]
mod linux_capture;
#[cfg(target_os = "linux")]
mod linux_layout;
mod media;
mod native_click;
mod page_origin;
mod pdf;
mod platform;
#[cfg(target_os = "macos")]
mod rules_macos;
mod storage;
pub mod window_shot;

pub use engine::{PageMenuChoice, TauriEngine};
pub use media::enable_media_capture;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str =
    "preview engine: Tauri child webviews with separated browser and preview storage";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
