//! Helper startup, package refresh and network-policy delivery for the Chrome connection.
//! Uses the parent connection state and owner; called by core startup, setup and browser switches.
#[cfg(all(feature = "your-safari", target_os = "macos"))]
use super::safari_startup;
use super::{ChromeControl, browser, helper_refresh, owner, safari_copies, setup};
use std::path::Path;

/// Repairs the running app's browser packages in the background, once at core startup.
pub(crate) async fn startup(shared: &std::sync::Arc<crate::core::Shared>) {
    #[cfg(all(feature = "your-safari", target_os = "macos"))]
    safari_startup::start(shared);
    safari_copies::refresh(shared).await;
    refresh_helper(shared).await;
}

/// Brings the helper copy Chrome starts up to this app version (see `helper_refresh`), on the blocking pool and
/// under the setup lock, so Set up and Remove never run beside it. A failure is logged and retried at the next
/// start or Connect; it never stops the app.
pub(crate) async fn refresh_helper(shared: &std::sync::Arc<crate::core::Shared>) {
    let chrome = &shared.chrome_control;
    let _held = chrome.lifecycle.lock().await;
    refresh_helper_held(chrome).await;
}

/// [`refresh_helper`] for a caller that holds the setup lock already.
pub(super) async fn refresh_helper_held(chrome: &ChromeControl) {
    let Some(state) = chrome.dir().map(Path::to_path_buf) else {
        return;
    };
    if !chrome.config.available() {
        return;
    }
    let config = chrome.config.clone();
    match tokio::task::spawn_blocking(move || helper_refresh::refresh(&config, &state)).await {
        Ok(Ok(helper_refresh::Refreshed::Replaced)) => {
            tracing::info!("Chrome Control: the helper Chrome starts was updated to this version");
        }
        Ok(Ok(_)) => {}
        Ok(Err(reason)) => {
            tracing::warn!(%reason, "Chrome Control: the helper Chrome starts could not be updated yet");
        }
        Err(join) => tracing::warn!(%join, "Chrome Control: the helper update task stopped"),
    }
}

/// Starts the installed helper without giving startup errors any secret-bearing details.
pub(crate) async fn start_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    start_owner_for(shared, None).await;
}

pub(super) async fn start_owner_for(
    shared: &std::sync::Arc<crate::core::Shared>,
    browser: Option<&str>,
) {
    let chrome = &shared.chrome_control;
    browser::refresh(&chrome.config).await;
    if chrome.config.available()
        && let Some(state) = chrome.dir()
        && let Err(reason) = setup::repair_registrations(&chrome.config, state, browser)
    {
        tracing::warn!(%reason, "Browser helper registration needs setup");
    }
    if let Err(reason) = owner::start(shared).await {
        tracing::warn!(%reason, "Chrome Control could not start");
    }
}

pub(crate) async fn stop_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    shared.chrome_control.owner.stop().await;
    super::profiles::disconnected(&shared.chrome_control);
    owner::publish(shared);
}
