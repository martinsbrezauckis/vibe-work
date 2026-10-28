//! Tauri wiring of the data-folder guard, with an existing-owner check before GUI initialization.
//! The host owns the OS lock and focus endpoint; this module only shows and focuses the main window.
//! A launch with `--attach` / `--attach-text -` ("Send to Vibe Work", `vw_host::hand_over`) hands its files or text
//! to the running app, or keeps them in this app's [`HandOverInbox`] when it becomes the owner.

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use vw_host::hand_over::{HandOver, HandOverInbox};
use vw_host::instance::InstanceStart;

const MESSAGE: &str =
    "Vibe Work is already running for this data folder. Open its existing window.";

/// This launch's own hand-over, read once from the arguments (standard input can be read only once).
fn launch_hand_over() -> Option<HandOver> {
    let cwd = std::env::current_dir().ok()?;
    HandOver::from_args(std::env::args_os().skip(1), &cwd, std::io::stdin())
}

pub(crate) fn context() -> Option<(tauri::Context<tauri::Wry>, Option<HandOver>)> {
    let context = tauri::generate_context!();
    let hand_over = launch_hand_over();
    if before_gui(context.config(), hand_over.as_ref()) {
        None
    } else {
        Some((context, hand_over))
    }
}

fn before_gui(config: &tauri::Config, hand_over: Option<&HandOver>) -> bool {
    let Some(data) = crate::startup::early_data_dir(config) else {
        return false;
    };
    match vw_host::instance::focus_existing(&data, hand_over) {
        Ok(Some(true)) => true,
        Ok(Some(false)) => {
            // A terminal launch may have no display even though its existing app does. Do not
            // initialize GTK merely to refuse the folder; desktop refusals use the dialog in setup.
            #[cfg(target_os = "linux")]
            if std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty())
                && std::env::var_os("WAYLAND_DISPLAY").is_none_or(|value| value.is_empty())
            {
                eprintln!("{MESSAGE}");
                return true;
            }
            false
        }
        Ok(None) | Err(_) => false,
    }
}

/// Claims the data folder (else hands this launch's hand-over and focus to its owner and exits). The owner keeps
/// hand-overs in its [`HandOverInbox`] (managed state, for the window's `hand_over_*` commands); on macOS the
/// services provider feeds the same inbox.
pub(crate) fn admit(
    app: &mut tauri::App,
    hand_over: Option<HandOver>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let data = crate::startup::data_dir(app.handle())?;
    let inbox = Arc::new(HandOverInbox::default());
    app.manage(inbox.clone());
    let handle = app.handle().clone();
    match vw_host::instance::start(
        &data,
        hand_over,
        move || {
            crate::main_window(&handle).is_some_and(|window| {
                if window.unminimize().is_err()
                    || window.show().is_err()
                    || window.set_focus().is_err()
                {
                    return false;
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                while std::time::Instant::now() < deadline {
                    if window.is_focused().unwrap_or(false) {
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                false
            })
        },
        {
            let inbox = inbox.clone();
            move |hand_over| inbox.push(hand_over)
        },
    )? {
        InstanceStart::First(guard) => {
            app.manage(guard);
            #[cfg(target_os = "macos")]
            crate::send_to::register_services(app.handle(), inbox);
            Ok(true)
        }
        InstanceStart::Existing { focused } => {
            if !focused {
                eprintln!("{MESSAGE}");
                let handle = app.handle().clone();
                app.dialog()
                    .message(MESSAGE)
                    .title("Vibe Work")
                    .show(move |_| handle.exit(0));
                return Ok(false);
            }
            app.handle().exit(0);
            Ok(false)
        }
    }
}
