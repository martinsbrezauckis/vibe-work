//! Wires every desktop notification to the installed identity and embedded app logo.
//! Uses Tauri's app data path (or the isolated live profile); persistence/registration live in vw-platform.
use tauri::{AppHandle, Runtime};

pub(crate) fn notifier<R: Runtime>(
    app: &AppHandle<R>,
    installed: bool,
) -> vw_platform::DesktopNotifier {
    let notifier = vw_platform::DesktopNotifier::new(app.config().identifier.clone(), installed);
    #[cfg(windows)]
    if installed {
        use tauri::Manager as _;
        let directory = crate::live_profile::root().and_then(|root| {
            root.map_or_else(
                || app.path().app_local_data_dir().map_err(|e| e.to_string()),
                |root| Ok(root.join("local-data")),
            )
        });
        let result = directory.and_then(|dir| {
            notifier
                .set_icon(
                    &dir.join("notification-icon.png"),
                    include_bytes!("../icons/128x128.png"),
                )
                .map_err(|e| e.to_string())
        });
        if let Err(error) = result {
            tracing::warn!(%error, "notification icon registration failed");
        }
    }
    notifier
}
