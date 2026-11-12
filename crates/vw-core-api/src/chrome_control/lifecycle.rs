//! Helper startup, package refresh and network-policy delivery for the Chrome connection.
//! Uses the parent connection state and owner; called by core startup, setup and browser switches.
#[cfg(all(feature = "your-safari", target_os = "macos"))]
use super::safari_startup;
use super::{ChromeControl, browser, helper_refresh, owner, safari_copies, setup};
use std::path::Path;
use std::sync::{PoisonError, atomic::Ordering};

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
        return;
    }
    // A new helper receives the compatibility allowance before dispatching browser work.
    send_local_network(shared).await;
}

/// Sends the always-enabled compatibility allowance to the helper and older extensions for every
/// tab the agent controls, and holds new actions until Chrome confirms. Without a running helper there is nothing
/// live to change.
/// The change counts only when the helper acknowledges it (its answer carries the new revision); otherwise
/// (a busy pipe, a lost connection) a task sends the current value again every 2 s until one is acknowledged or the
/// helper is gone. Meanwhile the view and `chrome_status` say the switch is not applied yet.
pub(crate) async fn send_local_network(shared: &std::sync::Arc<crate::core::Shared>) {
    if deliver_local_network(shared).await {
        return;
    }
    let chrome = &shared.chrome_control;
    if chrome.local_network_retry.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak = std::sync::Arc::downgrade(shared);
    tokio::spawn(async move {
        for _ in 0..900 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let Some(shared) = weak.upgrade() else { return };
            if !shared.chrome_control.owner.is_running() || deliver_local_network(&shared).await {
                shared
                    .chrome_control
                    .local_network_retry
                    .store(false, Ordering::SeqCst);
                owner::publish(&shared);
                return;
            }
        }
        if let Some(shared) = weak.upgrade() {
            shared
                .chrome_control
                .local_network_retry
                .store(false, Ordering::SeqCst);
        }
    });
}

/// Sends the current compatibility generation once; true when the helper acknowledged that change while it is still the
/// current one. Deliveries run one at a time from reading the state to the helper's answer
/// (`local_network_delivery`), in a task of their own: a caller that stops waiting does not end a delivery early,
/// so the next one is never sent while an older one may still reach the helper.
pub(super) async fn deliver_local_network(shared: &std::sync::Arc<crate::core::Shared>) -> bool {
    let shared = shared.clone();
    tokio::spawn(async move {
        let chrome = &shared.chrome_control;
        let _one_at_a_time = chrome.local_network_delivery.lock().await;
        let change = chrome.local_network_state();
        #[cfg(test)]
        chrome.held_after_sampling().await;
        let answer = chrome
            .owner
            .call(
                "chrome_status",
                serde_json::json!({"_netPolicyChange":{
                    "localNetwork":change.on,"generation":change.generation}}),
            )
            .await;
        let taken = matches!(&answer, owner::Answer::Helper(result)
            if result["structuredContent"]["revision"].is_u64());
        let mut acked = chrome
            .local_network_acked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if taken && chrome.local_network_state().generation == change.generation {
            *acked = Some(change.generation);
            return true;
        }
        drop(acked);
        tracing::debug!("Chrome Control did not acknowledge network compatibility now");
        false
    })
    .await
    .unwrap_or(false)
}

pub(crate) async fn stop_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    shared.chrome_control.owner.stop().await;
    super::profiles::disconnected(&shared.chrome_control);
    owner::publish(shared);
}
