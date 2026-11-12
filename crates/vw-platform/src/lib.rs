//! Operating-system edges: private files, credential stores, process identity, desktop integration,
//! native media and webviews. PTY/ConPTY live in vw-terminal.
//!
//! Depends on: platform libraries and the OS bindings listed in Cargo.toml; no workspace crates.
//! Called by: the core, host, browser, preview, vault and terminal services, and app wiring.
//! Invariant: only this crate may relax the unsafe-code ban, behind safe APIs with SAFETY comments.
//! Modules marked `#[allow(unsafe_code)]` below or in their own source are the unsafe boundary.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod a11y_bus;
pub mod atomic_file;
#[allow(unsafe_code)]
mod background;
pub mod browser;
pub mod checked_executable;
pub mod child_env;
pub mod clipboard;
pub use clipboard::content as clipboard_content;
pub mod screen_reader;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod search_directory;
pub use background::lower_thread_priority;
#[cfg(target_os = "macos")]
pub mod app_copies;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod clipboard_mac;
#[cfg(any(
    target_os = "macos",
    windows,
    all(target_os = "linux", feature = "startup-dialog")
))]
#[allow(unsafe_code)]
pub mod confirm_window;
#[cfg(all(feature = "webview", target_os = "macos"))]
#[allow(unsafe_code)]
pub mod content_rules;
pub mod edge_policy;
mod event_target;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod exec_inherit;
pub mod extension_registry;
pub mod final_path;
mod harden;
mod in_front;
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
pub mod process_command;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod safari_extension;
#[cfg(any(windows, test))]
mod tcp_listeners;
#[cfg(windows)]
pub use tcp_listeners::{TcpListener, windows_tcp_listeners};
mod remove_tree;
mod reveal;
pub mod send_to;
#[cfg(feature = "startup-dialog")]
pub mod startup_error;
mod system_version;
pub mod tray;
#[cfg(feature = "webview")]
pub mod webview;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod webview2_env;
#[cfg(feature = "webview")]
pub mod window_capture;
mod window_keys;

pub use event_target::keep_event_target_on_close;
#[cfg(target_os = "linux")]
pub use exec_inherit::stop_descriptor_inheritance;
pub use harden::{Hardening, harden_process, is_not_dumpable};
pub use in_front::own_app_in_front;
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
    replace_private_durable, set_aside, write_private_new,
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

/// Native output controls for voice permission capture.
#[cfg(all(feature = "webview", any(windows, target_os = "macos")))]
#[allow(unsafe_code)]
pub mod webview_audio;

#[cfg(feature = "video")]
#[allow(unsafe_code)]
pub mod video;

/// Restricted video helper launchers and native resource accounting.
#[cfg(feature = "video-sandbox")]
#[allow(unsafe_code)]
#[path = "video/isolation/mod.rs"]
pub mod video_sandbox;

#[cfg(feature = "proxy-resolver")]
#[allow(unsafe_code)]
pub mod proxy;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod credentials;

#[cfg(any(target_os = "macos", test))]
#[allow(unsafe_code)]
pub mod vault_keychain;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod vault_tpm;

#[cfg(feature = "safari-input")]
pub mod safari_input;
