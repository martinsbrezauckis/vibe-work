//! Operating-system edges: user-only files, OS notifications with click handling and
//! "show this folder in the file manager", "open this link in the browser", "open this file with
//! its default app / show it in its folder" without ever running a program, precise process
//! identity on macOS/Windows, process hardening for the key holder, the everyday spelling of a
//! canonical path (no Windows `\\?\` prefix in what people read); PTY/ConPTY live in vw-terminal (the OS
//! credential store is used by vw-vault).
//!
//! Depends on: nothing of ours (`thiserror` for errors, `url` for link checks, `dunce` for plain paths).
//! Called by: vw-gate, vw-debug-api (the per-run token descriptor), vw-procmon (process identity), app.
//! Invariant: the only crate allowed to relax the `unsafe_code` ban, and only behind a safe API with a SAFETY comment.
//! Only `process` (macOS `proc_pidinfo`, Windows process handles) and `webview` (feature `webview`: `WebView2` and
//! `WKWebView` screenshots), `native_click` (scoped webview input), `private_file` (Windows DACLs and
//! owners) and `window_keys` (the
//! main window's bare Alt tap) use it, and `content_rules` (macOS `WKContentRuleList` per preview webview,
//! with URL rules).

pub mod clipboard;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod clipboard_mac;
#[cfg(all(feature = "webview", target_os = "macos"))]
#[allow(unsafe_code)]
pub mod content_rules;
mod harden;
#[cfg(feature = "webview")]
#[allow(unsafe_code)]
pub mod native_click;
mod notify;
mod open_file;
mod open_url;
mod paths;
mod private_file;
mod process;
mod reveal;
#[cfg(feature = "webview")]
pub mod webview;
#[cfg(feature = "webview")]
pub mod window_capture;
mod window_keys;

pub use harden::{Hardening, harden_process, is_not_dumpable};
pub use notify::{DesktopNotifier, NotificationRequest, Notifier, NotifyError};
pub use open_file::{
    OpenClass, OpenFileError, file_open_class, looks_like_program, open_class, open_file,
    reveal_file,
};
pub use open_url::{
    LinkOpener, MAX_MAIL_LENGTH, MAX_URL_LENGTH, OpenUrlError, mail_url, open_mail_url,
    open_web_url, web_url,
};
pub use paths::{FolderFold, canonical_plain, folder_key, folder_key_for, plain_path, windows_key};
pub use private_file::{
    PrivateFileError, ensure_private_dir, is_user_only, replace_private, set_aside,
    write_private_new,
};
pub use process::{
    ProcessFacts, TerminateError, group_has_live_member, process_facts, terminate_verified,
};
pub use reveal::{Opener, RevealError, reveal_folder, reveal_target, windows_local_drive_path};
pub use window_keys::ignore_bare_alt;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "OS integration: keychain, paths, PTY, notifications";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
