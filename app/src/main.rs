//! Binary entry point. Hides the console window on Windows release builds and starts the app.
//! Depends on: `app`, `vw-platform` (native startup diagnostics). Called by: the operating system.

// Release builds on Windows run as a GUI program, without an extra console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if std::env::args_os().any(|arg| arg == "--clear-install-credentials") {
        let result = dirs::data_dir()
            .ok_or_else(|| "The app data folder is unavailable".to_owned())
            .and_then(|root| {
                vw_host::uninstall::clear_credentials(&root.join("app.vibework.desktop"))
            });
        return if result.is_ok() {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
    match app::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            vw_platform::startup_error::show(&error.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}
