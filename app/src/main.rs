//! Binary entry point. Hides the console window on Windows release builds and starts the app.
//! Depends on: the `app` library crate. Called by: the operating system.

// Release builds on Windows run as a GUI program, without an extra console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    match app::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Vibe Work failed to start: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
