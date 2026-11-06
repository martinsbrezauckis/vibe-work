//! App-owned Chrome connection. The chat access mode owns permissions.
//! Used by agent tools and Settings; pairing is independent of action authority.
mod addon;
pub(crate) mod agent_browser;
pub(crate) mod approval;
#[cfg(all(test, feature = "your-edge"))]
mod approval_tests;
mod availability;
pub(crate) mod batch;
mod browser;
mod browser_text;
pub(crate) mod builtin;
mod capabilities;
#[cfg(test)]
mod capability_tests;
pub(crate) mod check;
pub(crate) mod commands;
mod connection_notice;
mod content;
#[cfg(test)]
#[cfg(feature = "your-edge")]
mod edge_agent_tests;
#[cfg(test)]
#[cfg(feature = "your-edge")]
mod edge_lifecycle_tests;
mod edge_policy;
mod edge_state;
mod enroll;
mod enroll_browser;
mod escalation;
mod extension_install;
mod extension_registry;
mod firefox_commands;
pub(crate) mod gateway;
mod helper_refresh;
#[cfg(test)]
mod live_tests;
pub(crate) mod login;
mod mapping;
mod network;
#[cfg(test)]
mod observation_output_tests;
mod observation_support;
#[cfg(test)]
mod observation_support_tests;
#[cfg(test)]
mod observation_tests;
mod output;
mod output_batch;
mod output_cache;
mod output_file;
mod output_fragment;
#[cfg(test)]
mod output_guard_tests;
mod output_receipt;
mod output_status;
#[cfg(test)]
mod output_tests;
#[cfg(test)]
mod output_walk_tests;
mod owner;
#[cfg(test)]
mod owner_credential_tests;
mod pairing;
pub use pairing::PairingState;
mod person_stop;
#[cfg(test)]
mod person_stop_tests;
mod pipe;
mod probe;
#[cfg(test)]
mod profile_alias_tests;
mod profile_ids;
pub(crate) mod read_consent;
#[cfg(test)]
mod read_consent_tests;
#[cfg(test)]
mod read_output_tests;
mod receipt;
#[cfg(test)]
mod receipt_tests;
mod reconnect;
#[cfg(all(test, feature = "your-edge"))]
mod reconnect_tests;
#[cfg(test)]
mod registry_native_tests;
mod reopen;
#[cfg(test)]
mod security_tests;
mod selection;
#[cfg(all(test, feature = "your-edge"))]
mod session_routing_tests;
mod session_title;
mod session_title_output;
#[cfg(test)]
mod session_title_output_tests;
pub(crate) mod setup;
mod store;
mod tabs;
pub use tabs::ChromeTab;
#[cfg(test)]
mod switch_order_tests;
#[cfg(test)]
mod tab_pick_tests;
#[cfg(test)]
pub(crate) mod test_support;
mod work;
pub use addon::{AddonState, AddonView};
pub use commands::{ChromeControlRequest, ChromeControlView};
pub use selection::ChromeSelection;
use serde_json::json;
pub use setup::{ChromeControlConfig, ChromeSetupView};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
pub use work::ChromeWorkView;

/// The local-network switch with the number of the change that set it. The number grows with every change in this
/// run of the app; it goes with each delivery (`generation`), so the app records an acknowledgement only for the
/// current change and a helper that knows it refuses a change older than the one it holds.
#[derive(Clone, Copy)]
struct LocalNetwork {
    on: bool,
    generation: u64,
}
pub(crate) struct ChromeControl {
    dir: Option<PathBuf>,
    local_network_dir: PathBuf,
    enabled: AtomicBool,
    /// The person's local-network switch and its change number (see [`LocalNetwork`]).
    local_network: Mutex<LocalNetwork>,
    pub(crate) config: ChromeControlConfig,
    owner: owner::Owner,
    lifecycle: tokio::sync::Mutex<()>,
    generation: AtomicU64,
    /// A Connect Chrome click may open the unchanged extension's pairing page once.
    connect_requested: AtomicBool,
    /// An Edge setup/store click may open Edge's own pairing page once.
    edge_connect_requested: AtomicBool,
    /// A profile was connected with this setup at least once (kept in the state folder): a closed Chrome then
    /// reads "Connected when Chrome is open", never the pairing step again.
    paired_once: AtomicBool,
    /// Edge's earlier live connection, separate from Chrome and never permission to act.
    edge_paired_once: AtomicBool,
    /// Read-only look at Chrome's preferences (one probe at a time).
    prober: probe::Prober,
    /// Edge's own preferences cache; a Chrome result must never answer an Edge probe.
    edge_prober: probe::Prober,
    /// After Remove: the Chrome user data folders to check for an extension Chrome did not remove by itself.
    left_in_chrome: Mutex<Vec<PathBuf>>,
    removed_external: AtomicBool,
    work: Mutex<work::Work>,
    selections: Mutex<selection::Selections>,
    tasks: Mutex<std::collections::HashMap<String, String>>,
    output_cache: Mutex<output_cache::Cache>,
    chat_profiles: Mutex<std::collections::HashMap<String, String>>,
    /// Per chat, its session ids that ended because a turn ended (`reopen`): that chat may use them again.
    turn_ended: Mutex<std::collections::HashMap<String, std::collections::VecDeque<String>>>,
    /// The switch change the running helper acknowledged (`None`: not yet, or a new helper connection). A
    /// change stays pending, and is sent again, until the helper acknowledges that very change.
    local_network_acked: Mutex<Option<u64>>,
    /// One switch delivery at a time, from reading the switch to the helper's answer: a later delivery reads the
    /// switch only after an earlier one was answered, so an older value can never reach the helper after a newer one.
    local_network_delivery: tokio::sync::Mutex<()>,
    /// A delivery retry is running (one at a time).
    local_network_retry: AtomicBool,
    /// Last person Stop per browser and chat, bound to its turn and retained for racing replies.
    stopped_by_person: Mutex<std::collections::HashMap<String, Vec<person_stop::Stopped>>>,
    /// Tests: holds the next switch delivery right after it read the switch (signals, then waits to be released).
    #[cfg(test)]
    pub(super) hold_after_sampling: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
}
impl ChromeControl {
    pub(crate) fn new(vibe_home: &Path, config: ChromeControlConfig) -> Self {
        let dir = owner::state_dir(&config);
        let enabled = dir.as_deref().is_none_or(store::enabled);
        let local_network_dir = dir
            .clone()
            .unwrap_or_else(|| vibe_home.join("browser-settings"));
        let local_network = store::local_network(&local_network_dir);
        let paired_once = dir.as_deref().is_some_and(store::paired_once);
        let edge_paired_once = dir
            .as_deref()
            .is_some_and(|dir| store::paired_once(&dir.join("edge")));
        Self {
            dir,
            local_network_dir,
            enabled: AtomicBool::new(enabled),
            local_network: Mutex::new(LocalNetwork {
                on: local_network,
                generation: 0,
            }),
            config,
            owner: owner::Owner::default(),
            lifecycle: tokio::sync::Mutex::new(()),
            generation: AtomicU64::new(0),
            connect_requested: AtomicBool::new(false),
            edge_connect_requested: AtomicBool::new(false),
            paired_once: AtomicBool::new(paired_once),
            edge_paired_once: AtomicBool::new(edge_paired_once),
            prober: probe::Prober::default(),
            edge_prober: probe::Prober::default(),
            left_in_chrome: Mutex::new(Vec::new()),
            removed_external: AtomicBool::new(false),
            work: Mutex::new(work::Work::default()),
            selections: Mutex::new(selection::Selections::default()),
            tasks: Mutex::new(std::collections::HashMap::new()),
            output_cache: Mutex::new(output_cache::Cache::default()),
            chat_profiles: Mutex::new(std::collections::HashMap::new()),
            turn_ended: Mutex::new(std::collections::HashMap::new()),
            stopped_by_person: Mutex::new(std::collections::HashMap::new()),
            local_network_acked: Mutex::new(None),
            local_network_delivery: tokio::sync::Mutex::new(()),
            local_network_retry: AtomicBool::new(false),
            #[cfg(test)]
            hold_after_sampling: Mutex::new(None),
        }
    }
    /// Tests: waits here when a test asked to hold the delivery that just read the switch.
    #[cfg(test)]
    async fn held_after_sampling(&self) {
        let hold = self
            .hold_after_sampling
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((sampled, release)) = hold {
            let _ = sampled.send(());
            let _ = release.await;
        }
    }
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
    pub(crate) fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        if let Some(dir) = self.dir() {
            store::save_enabled(dir, enabled)?;
        }
        self.enabled.store(enabled, Ordering::SeqCst);
        Ok(())
    }
    /// Settings > Developer > Local network pages for the agent (off by default; Chrome Control only).
    pub(crate) fn local_network(&self) -> bool {
        self.local_network_state().on
    }
    fn local_network_state(&self) -> LocalNetwork {
        *self
            .local_network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
    /// Whether the running helper acknowledged the switch's current change.
    pub(crate) fn local_network_applied(&self) -> bool {
        *self
            .local_network_acked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            == Some(self.local_network_state().generation)
    }
    /// A new helper connection has acknowledged nothing yet.
    pub(super) fn forget_local_network_ack(&self) {
        *self
            .local_network_acked
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }
    /// Saves and sets the switch as a new change (also when the value is the same: it is delivered again).
    pub(crate) fn set_local_network(&self, on: bool) -> Result<(), String> {
        // Saved and set under one lock, so the saved value and the live one end on the same change.
        let mut state = self
            .local_network
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        store::save_local_network(&self.local_network_dir, on)?;
        *state = LocalNetwork {
            on,
            generation: state.generation + 1,
        };
        Ok(())
    }
    /// A profile is connected now: remembered for the setup's lifetime (`paired_once`).
    pub(super) fn mark_paired(&self) {
        if !self.paired_once.swap(true, Ordering::SeqCst)
            && let Some(dir) = self.dir()
            && let Err(reason) = store::save_paired_once(dir, true)
        {
            tracing::debug!(%reason, "the Chrome pairing record could not be saved");
        }
    }
    /// Chrome holds another owner: the earlier pairing no longer holds.
    pub(super) fn forget_paired(&self) {
        if self.paired_once.swap(false, Ordering::SeqCst)
            && let Some(dir) = self.dir()
        {
            let _ = store::save_paired_once(dir, false);
        }
    }
    pub(crate) fn reset(&self) {
        self.connect_requested.store(false, Ordering::SeqCst);
        self.edge_connect_requested.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .disconnected();
        self.chat_profiles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
    /// The chat's turn ended: its sessions end as with [`Self::end_chat`], and stay reopenable for this chat (`reopen`).
    /// A session already ended by the person's Stop is no longer in `tasks`, so it is not recorded.
    pub(crate) fn end_turn(&self, chat: &str) {
        let active: Vec<String> = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, owner)| *owner == chat)
            .map(|(id, _)| id.clone())
            .collect();
        if !active.is_empty() {
            let mut ended = self
                .turn_ended
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let ids = ended.entry(chat.to_owned()).or_default();
            for id in active {
                if !ids.contains(&id) {
                    ids.push_back(id);
                }
            }
            while ids.len() > 32 {
                ids.pop_front();
            }
        }
        // A racing reply still needs its turn's reason after interruption. A new turn has a different id.
        // Unbound idle rows keep the existing end-of-turn cleanup behavior.
        self.stopped_by_person
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(chat.to_owned())
            .and_modify(|held| held.retain(|stop| stop.turn.is_some()));
        self.end_chat(chat, false);
    }
    /// The person stopped these browser sessions (owner, from the helper's status): each browser stays
    /// stopped for the turn, and its turn-ended ids no longer reopen anything. Claims one interrupt per turn,
    /// including when both browsers report Stop together.
    fn person_stopped(&self, stopped: &[person_stop::Stopped]) -> Vec<person_stop::Stopped> {
        let mut claimed = Vec::new();
        let mut ended = self
            .turn_ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        let mut held = self
            .stopped_by_person
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for stop in stopped {
            let person_stop::Stopped { chat, browser, .. } = stop;
            if let Some(ids) = ended.get_mut(chat) {
                ids.retain(|id| {
                    work.profile_of(id).is_some_and(|profile| {
                        agent_browser::Browser::from_profile(profile) != *browser
                    })
                });
            }
            let browsers = held.entry(chat.clone()).or_default();
            if !browsers.iter().any(|held| held.turn == stop.turn) {
                claimed.push(stop.clone());
            }
            browsers.retain(|held| held.browser != *browser);
            browsers.push(stop.clone());
        }
        claimed
    }
    /// The person's Stop in Vibe Work (`stops::stopped`): the chat's Chrome work ends and its turn-ended
    /// ids no longer reopen anything; only the automatic turn end keeps them.
    pub(crate) fn stop_chat(&self, chat: &str) {
        self.turn_ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(chat);
        self.end_chat(chat, false);
    }
    /// The browser whose session the person stopped in this turn, independent of later requested profiles.
    pub(super) fn stopped_by_person(
        &self,
        chat: &str,
        browser: agent_browser::Browser,
        turn: Option<&str>,
    ) -> bool {
        self.stopped_by_person
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(chat)
            .is_some_and(|browsers| {
                browsers
                    .iter()
                    .any(|held| held.browser == browser && held.turn.as_deref() == turn)
            })
    }
    pub(crate) fn end_chat(&self, chat: &str, deleted: bool) {
        self.output_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear_chat(chat);
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .end(chat, deleted);
        let ids: Vec<_> = {
            let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
            let ids = tasks
                .iter()
                .filter(|(_, owner)| *owner == chat)
                .map(|(id, _)| id.clone())
                .collect();
            tasks.retain(|_, owner| owner != chat);
            ids
        };
        for id in ids {
            self.owner.control(json!({"action":"stop","sessionId":id}));
        }
        if deleted {
            self.stopped_by_person
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(chat);
            self.turn_ended
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(chat);
        }
        if deleted
            && let Some(profile) = self
                .chat_profiles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(chat)
        {
            self.owner
                .control(json!({"action":"delete","profileId":profile,"_clientSessionId":chat}));
        }
    }
}
pub(crate) fn end_chat(shared: &crate::core::Shared, chat: &str, deleted: bool) {
    shared.chrome_control.end_chat(chat, deleted);
    owner::publish(shared);
}
/// The person stopped the chat in Vibe Work: its Chrome work ends and is not reopened by an earlier id.
pub(crate) fn stop_chat(shared: &crate::core::Shared, chat: &str) {
    shared.chrome_control.stop_chat(chat);
    owner::publish(shared);
}
/// A turn of the chat ended (completed, failed or interrupted): its Chrome work ends; the chat may reopen it.
pub(crate) fn end_turn(shared: &crate::core::Shared, chat: &str) {
    shared.chrome_control.end_turn(chat);
    owner::publish(shared);
}
#[cfg(feature = "ts")]
pub(crate) fn declarations(config: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        ChromeControlRequest::decl(config),
        ChromeControlView::decl(config),
        ChromeWorkView::decl(config),
        ChromeSelection::decl(config),
        ChromeTab::decl(config),
        ChromeSetupView::decl(config),
        setup::SetupState::decl(config),
        PairingState::decl(config),
        browser::BrowserIssue::decl(config),
        addon::AddonView::decl(config),
        addon::AddonState::decl(config),
        addon::AddonAction::decl(config),
    ]
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
async fn refresh_helper_held(chrome: &ChromeControl) {
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
    let chrome = &shared.chrome_control;
    browser::refresh(&chrome.config).await;
    if chrome.config.available()
        && let Some(state) = chrome.dir()
        && let Err(reason) = setup::repair_registrations(&chrome.config, state, None)
    {
        tracing::warn!(%reason, "Browser helper registration needs setup");
    }
    if let Err(reason) = owner::start(shared).await {
        tracing::warn!(%reason, "Chrome Control could not start");
        return;
    }
    // A new helper learns the person's switch at once (it reports it to Chrome with every poll).
    send_local_network(shared).await;
}

/// Tells the helper the person's local-network switch: it sends it to Chrome, which applies it to every
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

/// Sends the switch's current change once; true when the helper acknowledged that change while it is still the
/// current one. Deliveries run one at a time from reading the switch to the helper's answer
/// (`local_network_delivery`), in a task of their own: a caller that stops waiting does not end a delivery early,
/// so the next one is never sent while an older one may still reach the helper.
async fn deliver_local_network(shared: &std::sync::Arc<crate::core::Shared>) -> bool {
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
        tracing::debug!("Chrome Control did not take the local-network switch now");
        false
    })
    .await
    .unwrap_or(false)
}

pub(crate) async fn stop_owner(shared: &std::sync::Arc<crate::core::Shared>) {
    shared.chrome_control.owner.stop().await;
    owner::publish(shared);
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn ending_a_chat_releases_its_tasks_and_deletion_forgets_its_window() {
        let chrome = ChromeControl::new(Path::new("."), ChromeControlConfig::default());
        chrome
            .tasks
            .lock()
            .unwrap()
            .extend([("a1".into(), "a".into()), ("b1".into(), "b".into())]);
        chrome
            .chat_profiles
            .lock()
            .unwrap()
            .insert("a".into(), "profile".into());
        chrome.end_chat("a", false);
        assert_eq!(chrome.tasks.lock().unwrap().len(), 1);
        assert!(chrome.tasks.lock().unwrap().contains_key("b1"));
        assert!(chrome.chat_profiles.lock().unwrap().contains_key("a"));
        chrome.end_chat("a", true);
        assert!(!chrome.chat_profiles.lock().unwrap().contains_key("a"));
    }
}

#[cfg(test)]
mod pairing_tests;

#[cfg(test)]
mod connected_schema_tests;

mod connected_schema;

pub(crate) mod installation;

#[cfg(all(test, feature = "your-edge"))]
mod browser_addon_tests;

#[cfg(test)]
mod status_actions_tests;

mod status_actions;

#[cfg(test)]
mod schema_budget_tests;
#[cfg(test)]
mod summary_support_tests;
