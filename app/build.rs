//! Build script: lets `tauri-build` validate tauri.conf.json and the capability files, embed the
//! window resources, and generate one `allow-<command>` permission per bridge command.
//!
//! The command list below is the whole attack surface the webview gets: the capability
//! `capabilities/main.json` grants exactly these permissions to the main window, and the private
//! test `app_security_config.rs` fails when the two lists differ or anything else is granted.
//! Called by cargo before compiling the crate.

/// The Tauri commands the main window may call. Keep in sync with `src/commands.rs`.
const BRIDGE_COMMANDS: &[&str] = &[
    "core_execute",
    "core_subscribe",
    "ui_control_next",
    "ui_control_reply",
    "debug_api_set",
    "cli_sharing",
    "debug_api_token_file",
    "host_subscribe",
    "pick_folder",
    "pick_documents",
    "pick_library_files",
    "export_page",
    "export_session",
    "reveal_folder",
    "open_link",
    "open_mail_link",
    "notify_session",
    "terminal_attach",
    "terminal_ack",
    "terminal_detach",
    "update_subscribe",
    "update_check",
    "update_download",
    "update_install",
    "update_set_auto_download",
    "update_open_release_page",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(BRIDGE_COMMANDS));
    if let Err(error) = tauri_build::try_build(attributes) {
        // A build script has no better channel than stderr and a failing exit code.
        eprintln!("tauri-build failed: {error:#}");
        std::process::exit(1);
    }
}
