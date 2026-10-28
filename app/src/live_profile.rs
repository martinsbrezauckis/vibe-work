//! Explicit isolated installed-app profile, shared by the native setup scripts.
//!
//! Depends on std, `serde_json` and vw-platform; startup validates it before any
//! host/window is created. This changes paths only, never vault command rights.
use std::path::PathBuf;

pub use vw_platform::live_profile::root;

/// Places the interview window's storage in its run (ephemeral on macOS).
pub fn window<R: tauri::Runtime, M: tauri::Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'_, R, M>,
    root: Option<PathBuf>,
) -> Result<tauri::WebviewWindowBuilder<'_, R, M>, String> {
    let Some(root) = root else {
        return Ok(builder);
    };
    let storage = vw_host::product::private_data_dir(Ok(root.join("webview")), None)?;
    let builder = builder.data_directory(storage).initialization_script(
        "Object.defineProperty(window, '__VW_TEST_PROFILE__', { value: true });",
    );
    // WKWebView cannot put a persistent store in a custom directory.
    #[cfg(target_os = "macos")]
    let builder = builder.incognito(true);
    Ok(builder)
}
