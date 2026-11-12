//! App-owned launch of a paired, closed browser before any action is sent. Navigation runs in Ask;
//! the gateway waits for the real helper connection. One launch is shared by concurrent chats, never their windows.
use super::{AddonState, agent_browser::Browser};
use crate::core::Shared;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);

enum Attempt {
    Pending(u64),
    Finished(Result<(), String>),
}
#[derive(Default)]
struct State {
    next: u64,
    attempts: [Option<Attempt>; 2],
}
/// A failed or timed-out launch is not repeated until a live connection is observed.
#[derive(Default)]
pub(super) struct Attempts(Mutex<State>);
impl Attempts {
    pub(super) fn connected(&self, shared: &Shared) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for browser in [Browser::Chrome, Browser::Edge] {
            if super::availability::live(shared, browser) {
                state.attempts[usize::from(browser == Browser::Edge)] = None;
            }
        }
    }
    pub(super) fn clear(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts = [None, None];
    }
    fn pending(&self, browser: Browser) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts[usize::from(browser == Browser::Edge)]
        .is_some()
    }
    fn begin(&self, shared: &Shared, browser: Browser) -> Option<u64> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let index = usize::from(browser == Browser::Edge);
        if state.attempts[index].is_some() || super::availability::live(shared, browser) {
            return None;
        }
        state.next += 1;
        let id = state.next;
        state.attempts[index] = Some(Attempt::Pending(id));
        Some(id)
    }
    fn finish(&self, browser: Browser, id: u64, result: Result<(), String>) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let attempt = &mut state.attempts[usize::from(browser == Browser::Edge)];
        if matches!(attempt, Some(Attempt::Pending(current)) if *current == id) {
            *attempt = Some(Attempt::Finished(result));
        }
    }
    fn error(&self, browser: Browser) -> Option<String> {
        match &self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attempts[usize::from(browser == Browser::Edge)]
        {
            Some(Attempt::Finished(Err(reason))) => Some(reason.clone()),
            _ => None,
        }
    }
}

fn needed(shared: &Shared, browser: Browser) -> bool {
    matches!(browser, Browser::Chrome | Browser::Edge)
        && shared.chrome_control.config.available()
        && browser.enabled(shared)
        && browser.reconnectable(shared)
        && !super::availability::live(shared, browser)
        && match browser.addon(shared).state {
            AddonState::ChromeClosed => true,
            AddonState::Connecting => shared.chrome_control.owner.starts.pending(browser),
            _ => false,
        }
}

fn active(shared: &Shared, session: &str, browser: Browser, generation: u64) -> Result<(), String> {
    super::reconnect::stopped(shared, session, browser)?;
    if generation != crate::stops::generation(shared, session)
        || shared.store.view(session).is_none()
    {
        return Err(super::person_stop::ended(browser));
    }
    super::check::authorize(
        shared,
        session,
        "chrome_session",
        &json!({"action":"open","profileId":browser.id()}),
    )?;
    Ok(())
}

fn failure(browser: Browser, code: &str, reason: &str) -> Value {
    let recovery = browser.words("Open {fullBrowser}, check that {extension} is on, then try again. Do not start the browser with process or shell tools.")
        .replace("{extension}", vw_vocabulary::label("workNames", "chromeExtension"));
    let instruction = format!("{reason} {recovery}");
    let mut answer = super::owner::own(&format!("Not run: {instruction}"), code).into_value();
    answer["structuredContent"]["addon"] = json!({
        "stateCode":if code == "BROWSER_CONNECT_TIMEOUT" { "connecting" } else { "chromeClosed" },
        "instruction":instruction,
    });
    json!({"vwMcpResult":answer})
}

/// Runs only after the gateway's current mode and ownership checks.
pub(super) async fn wait(
    shared: &Arc<Shared>,
    session: &str,
    browser: Browser,
) -> Result<Option<Value>, String> {
    if !needed(shared, browser) {
        return Ok(None);
    }
    let generation = crate::stops::generation(shared, session);
    active(shared, session, browser, generation)?;
    let deadline = tokio::time::Instant::now() + WAIT;
    if let Some(id) = shared.chrome_control.owner.starts.begin(shared, browser) {
        let weak = Arc::downgrade(shared);
        let chat = session.to_owned();
        tokio::task::spawn_blocking(move || {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let result = (|| {
                active(&shared, &chat, browser, generation)?;
                if super::availability::live(&shared, browser) {
                    return Ok(());
                }
                if !browser.reconnectable(&shared)
                    || !matches!(
                        browser.addon(&shared).state,
                        AddonState::ChromeClosed | AddonState::Connecting
                    )
                {
                    return Err("The browser connection changed before it could be opened".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("The browser launch did not finish in time".into());
                }
                super::browser::open_paired(&shared, browser)
            })();
            shared
                .chrome_control
                .owner
                .starts
                .finish(browser, id, result);
        });
    }
    loop {
        active(shared, session, browser, generation)?;
        let _ = tokio::time::timeout_at(
            deadline,
            shared.chrome_control.owner.call("chrome_status", json!({})),
        )
        .await;
        active(shared, session, browser, generation)?;
        if super::availability::live(shared, browser) {
            return Ok(None);
        }
        if let Some(reason) = shared.chrome_control.owner.starts.error(browser) {
            return Ok(Some(failure(
                browser,
                "BROWSER_LAUNCH_FAILED",
                &format!("{reason}."),
            )));
        }
        if !needed(shared, browser) {
            return Ok(super::availability::refusal(shared, browser, false));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Some(failure(browser, "BROWSER_CONNECT_TIMEOUT", &browser.words("Vibe Work asked {fullBrowser} to open, but its extension did not connect within 10 seconds."))));
        }
        tokio::time::sleep_until((tokio::time::Instant::now() + POLL).min(deadline)).await;
    }
}

#[cfg(test)]
mod wording_tests {
    use super::*;

    #[tokio::test]
    async fn ended_work_names_its_browser_and_keeps_the_recovery_instruction() {
        for browser in [Browser::Chrome, Browser::Edge] {
            let core =
                crate::offline_core::start(&format!("ended-browser-work-{}", browser.id())).await;
            let generation = crate::stops::generation(&core.shared, "a").wrapping_add(1);
            let error = active(&core.shared, "a", browser, generation).unwrap_err();
            assert_eq!(
                error,
                format!(
                    "Not run: this chat's {} work ended with this turn. Finish the reply; the browser can be used again in the next turn.",
                    browser.short_name()
                )
            );
            core.shutdown().await;
        }
    }
}
