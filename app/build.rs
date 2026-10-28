//! Build script: lets `tauri-build` validate tauri.conf.json and the capability files, embed the
//! window resources, and generate one `allow-<command>` permission per bridge command.
//!
//! The command list below is the whole attack surface the webview gets: the capability
//! `capabilities/main.json` grants exactly these permissions to the main window, and the private
//! test `app_security_config.rs` fails when the two lists differ or anything else is granted.
//! Called by cargo before compiling the crate.

mod release_pin;

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
    "pick_certificate",
    "pick_library_files",
    "export_page",
    "export_session",
    "reveal_folder",
    "open_link",
    "open_mail_link",
    "notify_session",
    "show_prompt",
    "terminal_attach",
    "terminal_ack",
    "terminal_detach",
    "update_subscribe",
    "update_check",
    "update_download",
    "update_install",
    "update_set_auto_download",
    "update_open_release_page",
    "hand_over_subscribe",
    "hand_over_take",
    "send_to",
];

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_DEBUG_ASSERTIONS");
    if let Err(reason) = release_pin::check_debug_assertions(
        &std::env::var("PROFILE").unwrap_or_default(),
        std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some(),
    ) {
        eprintln!("{reason}");
        std::process::exit(1);
    }
    println!("cargo:rerun-if-env-changed=VW_CHROME_HELPER_SHA256");
    if let Err(reason) = release_pin::check(
        &std::env::var("PROFILE").unwrap_or_default(),
        std::env::var("VW_CHROME_HELPER_SHA256").ok().as_deref(),
    ) {
        eprintln!("{reason}");
        std::process::exit(1);
    }
    println!("cargo:rerun-if-env-changed=VW_CHROME_HELPER_CDHASH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && let Err(reason) = release_pin::check_cdhash(
            &std::env::var("PROFILE").unwrap_or_default(),
            std::env::var_os("VW_CHROME_HELPER_SHA256").is_some(),
            std::env::var("VW_CHROME_HELPER_CDHASH").ok().as_deref(),
        )
    {
        eprintln!("{reason}");
        std::process::exit(1);
    }
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(BRIDGE_COMMANDS));
    if let Err(error) = tauri_build::try_build(attributes) {
        // A build script has no better channel than stderr and a failing exit code.
        eprintln!("tauri-build failed: {error:#}");
        std::process::exit(1);
    }
}
