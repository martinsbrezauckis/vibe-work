//! Which web permissions the main window grants.
//!
//! The main window is the trusted surface: it only ever shows the app's own bundle ([`crate::navigation`]), so a
//! permission request in it comes from Vibe Work's own UI. It needs exactly one: the **microphone**, for dictation
//! (`ui/src/features/voice/pcmCapture.ts` calls `getUserMedia({audio})`). Everything else (camera, screen capture,
//! geolocation, notifications through the web API, clipboard read, MIDI, sensors, ...) is refused. Child webviews
//! (previews, the Project and File viewers, the agent browser) refuse every permission in their own handlers.
//!
//! What granting means per OS (the app's handler answers before any engine prompt):
//! - **Windows (WebView2):** no `WebView2` prompt; the OS privacy switch "Let desktop apps access your microphone"
//!   is the one OS-level permission. Answering "default" would add a second, browser-style prompt that no other
//!   OS has; the trusted main window does not need it.
//! - **macOS (WKWebView):** `WebKit` grants; the OS then asks once (TCC) with the app's `NSMicrophoneUsageDescription`
//!   (`app/Info.plist`); a hardened-runtime build also needs the audio-input entitlement.
//! - **Linux (WebKitGTK):** `WebKitGTK` refuses any request nobody answers (wry connects its `permission-request`
//!   signal only when a handler is set); this grant is the only path, and there is no OS prompt.
//!   Without the handler dictation is denied; with it the microphone's audio can arrive.
//!   An older `WebKitGTK` also needs `enable-media-stream` (vw-webview `media`).
//!
//! Depends on: nothing. Called by: the app's main-window permission handler (`app/src/lib.rs`), which passes the
//! engine's permission kind as its stable display name (wry `PermissionKind`'s `Display`: `microphone`, `camera`,
//! `display-capture`, ...).
//! Invariants:
//! - Allow-list of one: only the exact name `microphone` is granted; an unknown or renamed kind is refused, so a
//!   rename upstream fails closed (dictation then reports "denied"), never open.

/// The permission kind the main window grants (wry's `PermissionKind::Microphone` display name).
pub const MAIN_WINDOW_GRANTED: &str = "microphone";

/// True when the main window grants the permission named `kind` (wry `PermissionKind` display name).
///
/// Only `microphone`; every other name, known or not, is refused.
///
/// # Examples
/// `no_run` like the workspace's other doctests: on Windows a runnable doctest is linked with every build script's
/// search path, and `tauri-build`'s 82-byte `msvcrt.lib` stub in the app's output then hides the real C runtime
/// (link error LNK2001 on Windows). The assertions run in the integration tests.
/// ```no_run
/// assert!(vw_host::media::main_window_grants("microphone"));
/// assert!(!vw_host::media::main_window_grants("camera"));
/// ```
#[must_use]
pub fn main_window_grants(kind: &str) -> bool {
    kind == MAIN_WINDOW_GRANTED
}
