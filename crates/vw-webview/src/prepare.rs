//! Prepares native child-view layout before the main page first paints.
//! Depends on Tauri and Linux GTK layout; called immediately after building the app's main window.
//! The main webview is never reparented when a later background view is created.

/// Prepares the main window's child-view layer (Linux only).
/// Call during window setup, before returning to the event loop and painting the main page.
///
/// # Errors
/// The native view could not be reached or its GTK parent is not the expected container.
pub fn prepare_child_views(window: &tauri::WebviewWindow<tauri::Wry>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let (reply, result) = std::sync::mpsc::channel();
        // Tauri executes with_webview inline on the GTK thread, as it does for media setup.
        window
            .with_webview(move |native| {
                let _ = reply.send(crate::linux_layout::prepare(&native.inner()));
            })
            .map_err(|error| error.to_string())?;
        result.try_recv().map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
        Ok(())
    }
}
