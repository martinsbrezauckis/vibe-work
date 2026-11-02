//! Build script: on Windows (MSVC) embeds the Common Controls 6 dependency into this package's own test and example
//! programs. It has no other job. Called by Cargo before compiling the crate.
//!
//! The library calls comctl32 functions that exist only in Common Controls 6 (`TaskDialogIndirect` in
//! `confirm_window`, `SetWindowSubclass` in `event_target`/`window_keys`). The app selects v6 through its own Tauri
//! manifest; a test or example program has none, so Windows refuses to start it (0xC0000139, entry point not found).
//! Cargo applies `rustc-link-arg` only to this package's linked targets (lib unit tests, integration tests,
//! examples): the rlib itself is not linked, and dependents such as the app never see these arguments.
use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Test/example programs have no app manifest; without Common Controls 6 they cannot start.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' \
             version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
