//! Bounded reconnect wait before browser actions. Uses trusted pairing; never retries an action already sent.
use super::{AddonState, agent_browser::Browser};
use crate::core::Shared;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

// No separate Chrome reconnect bound existed: the helper RPC deadline is 35 s, the batch deadline 25 s.
// A short shared wait leaves both budgets room for the action and its receipt.
const WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

/// One visible wake page per lost browser connection, reset only by a confirmed native connection.
#[derive(Default)]
pub(super) struct Wake(Mutex<[bool; 2]>);

impl Wake {
    fn take(&self, browser: Browser) -> bool {
        let mut attempts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let attempted = &mut attempts[usize::from(browser == Browser::Edge)];
        !std::mem::replace(attempted, true)
    }

    pub(super) fn observe(&self, data: &Value) {
        if let Some(profiles) = data["profiles"].as_array() {
            let mut attempts = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            for browser in [Browser::Chrome, Browser::Edge] {
                if profiles.iter().any(|p| {
                    p["connected"] == true
                        && p["profileId"]
                            .as_str()
                            .is_some_and(|id| browser.owns_profile(id))
                }) {
                    attempts[usize::from(browser == Browser::Edge)] = false;
                }
            }
        }
    }

    pub(super) fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = [false; 2];
    }
}

pub(super) fn retry(browser: Browser) -> Value {
    let mut view = super::addon::view_for(AddonState::Connecting, super::setup::ABSENT, browser);
    super::addon::reconnecting_for(&mut view, true, browser);
    let instruction = view.instruction;
    let mut answer =
        super::owner::own(&format!("Not run: {instruction}"), "EXTENSION_UNRESPONSIVE")
            .into_value();
    // No action was sent during the reconnect wait. A later retry can use the recovered connection;
    // the instruction asks for the person's help only if the connection stays down.
    answer["structuredContent"]["retryable"] = json!(true);
    answer["structuredContent"]["addon"] = json!({
        "state":view.label, "stateCode":view.state, "instruction":instruction,
    });
    json!({"vwMcpResult":answer})
}

/// The helper has not accepted an action yet and its session is still reconnecting.
pub(super) fn pending(browser: Browser) -> Value {
    let instruction = format!(
        "{} is reconnecting; try again in a few seconds",
        browser.name()
    );
    let mut answer =
        super::owner::own(&format!("Not run: {instruction}"), "SESSION_NOT_READY").into_value();
    answer["structuredContent"]["retryable"] = json!(true);
    answer["structuredContent"]["addon"] = json!({
        "state":"Reconnecting", "stateCode":AddonState::Connecting, "instruction":instruction,
    });
    json!({"vwMcpResult":answer})
}

pub(super) fn stopped(shared: &Shared, session: &str, browser: Browser) -> Result<(), String> {
    if shared.chrome_control.stopped_by_person(
        session,
        browser,
        super::person_stop::turn(shared, session).as_deref(),
    ) {
        return Err(super::person_stop::refusal(browser));
    }
    Ok(())
}

pub(super) async fn wait(
    shared: &Arc<Shared>,
    session: &str,
    browser: Browser,
) -> Result<Option<Value>, String> {
    if !browser.reconnectable(shared) || browser.addon(shared).state != AddonState::Connecting {
        return Ok(None);
    }
    let generation = crate::stops::generation(shared, session);
    let deadline = tokio::time::Instant::now() + WAIT;
    tracing::debug!(browser = browser.id(), "Browser reconnect wait started");
    active(shared, session, browser, generation)?;
    wake(shared, session, browser, generation, deadline).await;
    loop {
        active(shared, session, browser, generation)?;
        if !browser.reconnectable(shared) {
            return Ok(super::availability::refusal(shared, browser, false));
        }
        match browser.addon(shared).state {
            AddonState::SwitchedOff
            | AddonState::UpdateNeeded
            | AddonState::OtherOwner
            | AddonState::NotPaired
            | AddonState::Unavailable => {
                return Ok(super::availability::refusal(shared, browser, false));
            }
            _ => {}
        }
        let status = tokio::time::timeout_at(
            deadline,
            shared.chrome_control.owner.call("chrome_status", json!({})),
        )
        .await;
        active(shared, session, browser, generation)?;
        if status.is_ok() && super::availability::live(shared, browser) {
            tracing::debug!(browser = browser.id(), "Browser reconnect wait completed");
            return Ok(None);
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::debug!(browser = browser.id(), "Browser reconnect wait expired");
            return Ok(Some(retry(browser)));
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + POLL).min(deadline)).await;
    }
}

fn known_extension(shared: &Shared, browser: Browser) -> bool {
    if shared.chrome_control.config.is_macos() {
        // The extension's earlier pairing is our evidence; its protected profile cannot be inspected.
        browser.was_paired(shared)
    } else {
        !super::installation::profiles(shared, browser).is_empty()
    }
}

async fn wake(
    shared: &Arc<Shared>,
    session: &str,
    browser: Browser,
    generation: u64,
    deadline: tokio::time::Instant,
) {
    let config = &shared.chrome_control.config;
    let injected = match browser {
        Browser::Chrome => config.launcher.is_some(),
        Browser::Edge => config.edge_launcher.is_some(),
        Browser::Safari | Browser::Firefox => return,
    };
    if (!injected && !known_extension(shared, browser))
        || !shared.chrome_control.owner.wake.take(browser)
    {
        return;
    }
    let weak = Arc::downgrade(shared);
    let session = session.to_owned();
    let launch = tokio::task::spawn_blocking(move || {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let config = &shared.chrome_control.config;
        if !injected
            && !match browser {
                Browser::Chrome => super::browser::running(config),
                Browser::Edge => super::browser::edge_running(config),
                Browser::Safari | Browser::Firefox => false,
            }
        {
            return;
        }
        if active(&shared, &session, browser, generation).is_err()
            || tokio::time::Instant::now() >= deadline
        {
            return;
        }
        let config = &shared.chrome_control.config;
        let url = format!(
            "chrome-extension://{}/ui.html?surface=panel",
            config.extension_id
        );
        // The extension page sends a runtime message, waking MV3 without debugger access or new permissions.
        // This does not change the requested profile or replay an action whose outcome is unknown.
        let result = match browser {
            Browser::Chrome => super::browser::open(config, &url),
            Browser::Edge => super::browser::open_edge(config, &url),
            Browser::Safari | Browser::Firefox => return,
        };
        if let Err(error) = result {
            tracing::debug!(%error, browser = browser.id(), "Browser wake failed");
        }
    });
    let _ = tokio::time::timeout_at(deadline, launch).await;
}

fn active(shared: &Shared, session: &str, browser: Browser, generation: u64) -> Result<(), String> {
    stopped(shared, session, browser)?;
    if generation != crate::stops::generation(shared, session)
        || shared.store.view(session).is_none()
    {
        return Err(super::person_stop::ended(browser));
    }
    Ok(())
}

#[cfg(all(test, feature = "your-edge"))]
#[path = "reconnect_wake_tests.rs"]
mod wake_tests;
