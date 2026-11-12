//! Per-browser availability and helper delivery. Settings owns the switches; disabling one fences only its work.
//! The helper receives the same snapshot at startup and on changes; access modes remain the action authority.
use super::{ChromeControl, agent_browser::Browser};
use crate::{CommandResult, Core, CoreError, core::Shared};
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError, atomic::Ordering};

impl ChromeControl {
    pub(crate) fn enabled(&self) -> bool {
        self.switches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .chrome
    }
    pub(super) fn edge_enabled(&self) -> bool {
        self.switches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .edge
    }
    pub(super) fn firefox_enabled(&self) -> bool {
        self.switches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .firefox
    }
    pub(crate) fn any_enabled(&self) -> bool {
        let switches = self.switches.lock().unwrap_or_else(PoisonError::into_inner);
        switches.chrome
            || (cfg!(feature = "your-edge") && switches.edge)
            || (cfg!(feature = "your-firefox") && switches.firefox)
    }
    pub(crate) fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.set_browser_enabled(Browser::Chrome, enabled)
    }
    pub(super) fn set_browser_enabled(
        &self,
        browser: Browser,
        enabled: bool,
    ) -> Result<(), String> {
        let mut saved = self.switches.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = *saved;
        let target = if browser == Browser::Edge {
            &mut next.edge
        } else if browser == Browser::Firefox {
            &mut next.firefox
        } else {
            &mut next.chrome
        };
        let changed = *target != enabled;
        *target = enabled;
        if let Some(dir) = self.dir() {
            next.save(dir)?;
        }
        *saved = next;
        if changed {
            self.switch_generation.fetch_add(1, Ordering::SeqCst);
            if !enabled {
                self.owner.approvals.cancel_browser(browser);
                let generation = if browser == Browser::Edge {
                    &self.edge_generation
                } else if browser == Browser::Firefox {
                    &self.firefox_generation
                } else {
                    &self.chrome_generation
                };
                generation.fetch_add(1, Ordering::SeqCst);
                if browser == Browser::Firefox {
                    self.local_documents
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clear();
                }
            }
        }
        Ok(())
    }
    pub(super) fn browser_generation(&self, browser: Browser) -> u64 {
        match browser {
            Browser::Chrome => &self.chrome_generation,
            Browser::Firefox => &self.firefox_generation,
            Browser::Edge => &self.edge_generation,
            Browser::Safari => &self.safari_generation,
        }
        .load(Ordering::SeqCst)
    }
}

pub(super) fn payload(shared: &Shared) -> Value {
    let chrome = &shared.chrome_control;
    let switches = *chrome
        .switches
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let disabled = chrome
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .disabled();
    let mut payload = json!({"chrome":switches.chrome,"edge":cfg!(feature = "your-edge") && switches.edge,
        "safari":super::safari::enabled(shared),"firefox":cfg!(feature = "your-firefox") && switches.firefox,"generation":chrome.switch_generation.load(Ordering::SeqCst)});
    if !disabled.is_empty() {
        payload["disabledProfiles"] = json!(disabled);
    }
    payload
}

pub(super) async fn synchronize(shared: &Shared, pipe: &mut super::pipe::Pipe) -> Result<(), ()> {
    let applied = pipe
        .rpc("owner.browserAvailability", payload(shared))
        .await?;
    if applied["applied"] == true {
        Ok(())
    } else {
        Err(())
    }
}

pub(super) async fn deliver(shared: &Arc<Shared>) -> Result<(), CoreError> {
    let chrome = &shared.chrome_control;
    if !chrome.owner.is_running() {
        return Ok(());
    }
    let result = chrome
        .owner
        .rpc("owner.browserAvailability", payload(shared))
        .await;
    if result.is_some_and(|value| value["applied"] == true) {
        return Ok(());
    }
    // An unacknowledged switch cannot leave an old helper executing with stale availability.
    super::stop_owner(shared).await;
    Err(super::commands::invalid(
        "Browser setting saved. The browser helper could not apply it.",
    ))
}

pub(super) fn stop_work(shared: &Shared, browser: Browser) {
    let chrome = &shared.chrome_control;
    let active = {
        let mut tasks = chrome.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        let mut work = chrome.work.lock().unwrap_or_else(PoisonError::into_inner);
        let selected = |id: &str| {
            work.profile_of(id)
                .map(str::to_owned)
                .or_else(|| super::profile_bindings::session_profile(chrome, id))
                .as_deref()
                .is_some_and(|profile| browser.owns_profile(profile))
        };
        let active: Vec<_> = tasks
            .iter()
            .filter(|(id, _)| selected(id))
            .map(|(id, chat)| (id.clone(), chat.clone()))
            .collect();
        for (id, chat) in &active {
            tasks.remove(id);
            work.ask_stop(id, chat);
        }
        active
    };
    for (id, _) in active {
        chrome
            .owner
            .control(json!({"action":"stop","sessionId":id}));
    }
}

impl Core {
    pub(super) async fn browser_enabled(
        &self,
        browser: Browser,
        enabled: bool,
    ) -> Result<CommandResult, CoreError> {
        let chrome = &self.shared.chrome_control;
        let _held = chrome.lifecycle.lock().await;
        chrome
            .set_browser_enabled(browser, enabled)
            .map_err(|reason| super::commands::invalid(&reason))?;
        if !enabled {
            stop_work(&self.shared, browser);
            if matches!(browser, Browser::Chrome | Browser::Edge) {
                let enrollment = if browser == Browser::Edge {
                    super::enroll_browser::Browser::Edge
                } else {
                    super::enroll_browser::Browser::Chrome
                };
                enrollment
                    .requested(&self.shared)
                    .store(false, Ordering::SeqCst);
                enrollment
                    .enroll(&self.shared)
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .connection_ended(enrollment);
            }
        }
        let delivered = deliver(&self.shared).await;
        if chrome.any_enabled() || super::safari::enabled(&self.shared) {
            super::start_owner(&self.shared).await;
        } else {
            super::stop_owner(&self.shared).await;
        }
        super::owner::publish(&self.shared);
        delivered?;
        Ok(CommandResult::ChromeControl {
            view: super::commands::view(&self.shared),
            offer: None,
        })
    }
}
