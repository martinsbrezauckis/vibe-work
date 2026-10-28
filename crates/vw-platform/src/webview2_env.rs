//! Windows: the `WebView2` loader's environment variables are removed from Vibe Work's own process at startup, before
//! any `WebView2` environment exists.
//!
//! Why: the loader reads `WEBVIEW2_*` variables when each environment is created and lets them replace or extend what
//! the app asks for (Microsoft Learn, Win32 globals, `CreateCoreWebView2EnvironmentWithOptions`:
//! `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`, `WEBVIEW2_USER_DATA_FOLDER`, `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`,
//! `WEBVIEW2_CHANNEL_SEARCH_KIND`, `WEBVIEW2_RELEASE_CHANNELS`; the script-debugger pair
//! `WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER` / `WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER`; the older
//! `WEBVIEW2_RELEASE_CHANNEL_PREFERENCE` on the .NET page). `ADDITIONAL_BROWSER_ARGUMENTS` is APPENDED to ours and
//! Chromium uses the last copy of a switch, so a `--proxy-server` or `--no-proxy-server` there takes the previews off
//! the network guard's proxy. The user can set these with `setx` (it writes `HKCU\Environment`, which the user owns),
//! and so can anything that runs as the user, the agent's own shell included: that made the guard's proxy something the
//! agent could switch off for the next launch. So every variable whose name starts with `WEBVIEW2_` (any case, as
//! Windows treats names; covers names Microsoft adds later) is removed from this process. Admin policies under
//! `Software\Policies\Microsoft\Edge\WebView2` (a standard user can read but not write them) stay in force; the engine
//! refuses previews when one of them makes the guard impossible (`webview` `user_data_folder_override`,
//! `policy_arguments_change_proxy`).
//!
//! Children: the variables are gone from this process, so from everything it starts. They are not handed back: the
//! supervisor's environment allowlist (`vw-supervisor` `INHERITED`) never had them (Vibe never saw them), and a
//! terminal the user opens inside Vibe Work starts without them too (not widened on purpose); the user's own setting in
//! `HKCU\Environment` is untouched for every other program.
//!
//! Development builds only: `VW_DEV_KEEP_WEBVIEW2_ENV=1` keeps them (the Windows `WebDriver` e2e needs msedgedriver's
//! `WEBVIEW2_USER_DATA_FOLDER` and `--remote-debugging-port` to find the app; previews are refused there anyway).
//!
//! Depends on: std. Called by: the app (`initialize_process`, first thing).

/// The prefix of the `WebView2` loader's variables.
pub const PREFIX: &str = "WEBVIEW2_";

/// The development-build switch that keeps the variables.
pub const KEEP_VAR: &str = "VW_DEV_KEEP_WEBVIEW2_ENV";

/// What [`remove_loader_variables`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoaderVariables {
    /// The names removed (never the values), sorted.
    pub removed: Vec<String>,
    /// The names kept because of [`KEEP_VAR`] (development builds only), sorted.
    pub kept: Vec<String>,
}

/// Removes every `WEBVIEW2_*` variable (name compared without case) from this process's environment, except in a
/// development build with `VW_DEV_KEEP_WEBVIEW2_ENV=1`. Call before any `WebView2` environment is created.
#[must_use]
#[allow(unsafe_code)]
pub fn remove_loader_variables() -> LoaderVariables {
    let mut names: Vec<std::ffi::OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| {
            name.to_str().is_some_and(|n| {
                n.len() >= PREFIX.len() && n[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
            })
        })
        .collect();
    names.sort();
    let shown = |names: &[std::ffi::OsString]| -> Vec<String> {
        names
            .iter()
            .map(|n| n.to_string_lossy().into_owned())
            .collect()
    };
    if cfg!(debug_assertions) && std::env::var_os(KEEP_VAR).is_some_and(|v| v == "1") {
        return LoaderVariables {
            removed: Vec::new(),
            kept: shown(&names),
        };
    }
    for name in &names {
        // SAFETY: the Rust standard library documents `remove_var` as "always sound to call on Windows, in
        // single-threaded and multi-threaded programs"; this module is Windows-only.
        unsafe { std::env::remove_var(name) };
    }
    LoaderVariables {
        removed: shown(&names),
        kept: Vec::new(),
    }
}
