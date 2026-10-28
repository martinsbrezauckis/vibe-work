//! Operating-system edges: user-only files, OS notifications with click handling and
//! "show this folder in the file manager", "open this link in the browser", "open this file with
//! its default app / show it in its folder" without ever running a program, precise process
//! identity on macOS/Windows, process hardening for the key holder, the everyday spelling of a
//! canonical path (no Windows `\\?\` prefix in what people read), the startup accessibility bus check
//! (Linux) and keeping that check's run-only setting out of the programs the app starts (`child_env`), and the
//! "Send to Vibe Work" right-click entries (`send_to`), and the system version (`system_version`: macOS and the
//! Windows build; `glibc_version` on Linux);
//! PTY/ConPTY live in vw-terminal (the OS credential store is used by vw-vault).
//!
//! Depends on: nothing of ours (`thiserror` for errors, `url` for link checks, `dunce` for plain paths, `dbus`
//! for the accessibility bus check).
//! Called by: vw-gate, vw-debug-api (the per-run token descriptor), vw-procmon (process identity), vw-terminal
//! (`child_env`), app.
//! Invariant: the only crate allowed to relax the `unsafe_code` ban, and only behind a safe API with a SAFETY comment.
//! Only `process` (macOS `proc_pidinfo`, Windows process handles) and `webview` (feature `webview`: `WebView2` and
//! `WKWebView` screenshots), `native_click` (scoped webview input), `private_file` (Windows DACLs and
//! owners), `background` (per-thread scheduling), `window_keys` (the main window's bare Alt tap),
//! `a11y_bus` (Linux: `NO_AT_BRIDGE` for one run, set only while the process has one thread),
//! `content_rules` (macOS `WKContentRuleList` per preview webview, with URL rules), `web_transport` (macOS: the
//! WebTransport preference of one preview webview), `exec_inherit` (Linux: close-on-exec for inherited
//! descriptors before the restart after an update), `send_to` (the Windows Send to shortcut through COM, the
//! macOS services provider), `confirm_window` (the macOS `NSAlert` sheet and the Windows task dialog of the app's
//! confirmations), `remove_tree` (Windows: file deletion that preserves shared read-only attributes), `kill_job` (Windows: a program in a
//! kill-on-close job, for the Git Bash wrapper) and
//! `webview2_env` (Windows: the `WebView2` loader's variables removed from this process at startup) use it.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod a11y_bus;
#[allow(unsafe_code)]
mod background;
pub mod checked_executable;
pub mod child_env;
pub mod clipboard;
pub use background::lower_thread_priority;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod clipboard_mac;
#[cfg(any(target_os = "macos", windows))]
#[allow(unsafe_code)]
pub mod confirm_window;
#[cfg(all(feature = "webview", target_os = "macos"))]
#[allow(unsafe_code)]
pub mod content_rules;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod exec_inherit;
mod harden;
pub mod keep_awake;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod kill_job;
#[cfg(feature = "webview")]
pub mod live_profile;
#[cfg(feature = "webview")]
#[allow(unsafe_code)]
pub mod native_click;
mod notify;
mod open_file;
mod open_url;
mod paths;
mod private_file;
mod process;
mod remove_tree;
mod reveal;
pub mod send_to;
mod system_version;
#[cfg(all(feature = "webview", target_os = "macos"))]
#[allow(unsafe_code)]
pub mod web_transport;
#[cfg(feature = "webview")]
pub mod webview;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod webview2_env;
#[cfg(feature = "webview")]
pub mod window_capture;
mod window_keys;

#[cfg(target_os = "linux")]
pub use exec_inherit::stop_descriptor_inheritance;
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
    PrivateFileError, create_private_new, ensure_private_dir, is_user_only, replace_private,
    set_aside, write_private_new,
};
pub use process::{
    ProcessFacts, TerminateError, group_has_live_member, parse_procargs, process_argv,
    process_facts, terminate_verified,
};
pub use remove_tree::remove_owned_tree;
pub use reveal::{Opener, RevealError, reveal_folder, reveal_target, windows_local_drive_path};
pub use system_version::{glibc_version, system_version};
pub use window_keys::ignore_bare_alt;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "OS integration: keychain, paths, PTY, notifications";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
