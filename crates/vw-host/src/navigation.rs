//! Navigation policy of the main window (the main window is a trusted surface).
//!
//! The main webview may only ever show the app's own bundle. A link, a form post, a script or a
//! redirect that tries to take it to another origin is refused here; web content belongs in the
//! isolated browser engine (a later block), never in the window that holds the Tauri bridge.
//!
//! Depends on: `url`. Called by: the app's navigation hook (`app/src/lib.rs`).
//! Invariants:
//! - Allow-list, not block-list: anything not named is refused, including `file:`, `data:`,
//!   `javascript:`, `blob:` and every remote origin.
//! - Hosts are compared exactly (no suffix matching, so `tauri.localhost.evil.example` is out).
//! - The dev server origin is only allowed when the caller says this is a development build.

use url::Url;

/// The dev server the Vite config pins (`ui/vite.config.ts`, `strictPort`).
const DEV_SERVER_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
const DEV_SERVER_PORT: u16 = 1420;

/// True when the main window may navigate to `target`.
///
/// Allowed: the packaged app origin (`tauri://localhost` on macOS and Linux,
/// `http(s)://tauri.localhost` on Windows) and `about:blank`; in a development build also the dev
/// server on `127.0.0.1:1420` / `localhost:1420`.
#[must_use]
pub fn navigation_allowed(target: &Url, dev_build: bool) -> bool {
    match target.scheme() {
        "about" => target.as_str() == "about:blank",
        "tauri" => target.host_str() == Some("localhost") && target.port().is_none(),
        "http" | "https" => {
            let host = target.host_str();
            if host == Some("tauri.localhost") && target.port().is_none() {
                return true;
            }
            dev_build
                && target.scheme() == "http"
                && host.is_some_and(|h| DEV_SERVER_HOSTS.contains(&h))
                && target.port() == Some(DEV_SERVER_PORT)
        }
        _ => false,
    }
}
