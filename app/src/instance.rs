//! Tauri wiring of the data-folder guard, with an existing-owner check before GUI initialization.
//! The host owns the OS lock and focus endpoint; this module only shows and focuses the main window.

use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use vw_host::instance::InstanceStart;

const MESSAGE: &str =
    "Vibe Work is already running for this data folder. Open its existing window.";

pub(crate) fn context() -> Option<tauri::Context<tauri::Wry>> {
    let context = tauri::generate_context!();
    if before_gui(context.config()) {
        None
    } else {
        Some(context)
    }
}

fn before_gui(config: &tauri::Config) -> bool {
    let Some(data) = crate::startup::early_data_dir(config) else {
        return false;
    };
    match vw_host::instance::focus_existing(&data) {
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

pub(crate) fn admit(app: &mut tauri::App) -> Result<bool, Box<dyn std::error::Error>> {
    let data = crate::startup::data_dir(app.handle())?;
    let handle = app.handle().clone();
    match vw_host::instance::start(&data, move || {
        handle.get_webview_window("main").is_some_and(|window| {
            if window.unminimize().is_err() || window.show().is_err() || window.set_focus().is_err()
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
    })? {
        InstanceStart::First(guard) => {
            app.manage(guard);
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
