//! App-owned Chrome connection. The chat access mode owns permissions.
//! Used by agent tools and Settings; pairing is independent of action authority.
mod action_tracking;
mod addon;
#[cfg(test)]
mod agent_profile_text_tests;
pub(crate) mod safari;
mod safari_copies;
mod safari_package;
mod safari_pairing;
#[cfg(any(all(feature = "your-safari", target_os = "macos"), test))]
mod safari_startup;
pub use safari::SafariView;
pub(crate) mod agent_browser;
pub(crate) mod approval;
#[cfg(all(test, feature = "your-edge"))]
mod approval_tests;
mod availability;
pub(crate) mod batch;
mod batch_wait;
mod browser;
pub(crate) mod browser_start;
#[cfg(all(test, feature = "your-edge"))]
mod browser_start_tests;
mod browser_text;
pub(crate) mod builtin;
pub(crate) mod builtin_share;
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
use vw_platform::extension_registry;
#[cfg(test)]
#[cfg(feature = "your-firefox")]
mod firefox_boundary_tests;
mod firefox_commands;
#[cfg(all(test, feature = "your-firefox"))]
mod firefox_hidden_tests;
mod firefox_package;
mod firefox_probe;
mod firefox_setup;
mod firefox_sharing;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod firefox_switch_tests;
pub(crate) mod gateway;
mod helper_refresh;
#[cfg(all(test, feature = "your-edge"))]
mod inactive_profile_tests;
mod lifecycle;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
use lifecycle::deliver_local_network;
use lifecycle::{refresh_helper_held, start_owner_for};
pub(crate) use lifecycle::{send_local_network, start_owner, startup, stop_owner};
mod local_documents;
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
#[cfg(test)]
mod page_tool_output_tests;
mod pairing;
mod pairing_approval;
pub use pairing_approval::BROWSER_PAIRING_DIALOG_METHOD;
#[cfg(test)]
mod pairing_approval_tests;
#[cfg(test)]
mod pairing_removed_tests;
pub use pairing::PairingState;
#[cfg(test)]
mod ask_policy_tests;
#[cfg(test)]
mod close_consent_tests;
pub(crate) mod interaction_consent;
#[cfg(test)]
mod interaction_consent_tests;
mod person_stop;
#[cfg(test)]
mod person_stop_tests;
mod pipe;
mod probe;
mod profile_agent;
#[cfg(test)]
mod profile_alias_tests;
mod profile_bindings;
mod profile_commands;
mod profile_seeding;
mod profile_selection;
pub use profile_selection::ChatBrowserProfile;
mod profiles;
pub use profiles::{BrowserProfile, ProfileState};
mod browser_data;
#[cfg(test)]
mod profile_card_agent_tests;
#[cfg(test)]
mod profile_choice_tests;
mod profile_folders;
mod profile_ids;
mod profile_names;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_access_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_macos_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_tests;
mod profile_presence;
#[cfg(all(test, feature = "your-edge"))]
mod profile_presence_tests;
mod profile_reason;
#[cfg(all(test, feature = "your-edge"))]
mod profile_reinstall_tests;
#[cfg(test)]
mod profile_retire_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_scope_tests;
mod profile_storage;
pub(crate) mod read_consent;
#[cfg(test)]
mod read_consent_tests;
#[cfg(test)]
mod read_output_tests;

#[cfg(test)]
mod output_secret_tests;
mod receipt;
#[cfg(test)]
mod receipt_tests;
mod reconnect;
#[cfg(all(test, feature = "your-edge"))]
mod reconnect_tests;
#[cfg(test)]
mod registry_native_tests;
mod remove;
#[cfg(all(test, feature = "your-safari"))]
mod safari_action_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_supersede_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_tests;
#[cfg(test)]
mod secret_policy_tests;
#[cfg(test)]
mod security_tests;
mod selection;
mod session_activity;
#[cfg(test)]
mod session_lifetime_tests;
#[cfg(test)]
mod session_modal_tests;
#[cfg(all(test, feature = "your-edge"))]
mod session_routing_tests;
mod session_start;
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
pub use selection::{ChromeSelection, ChromeShareKind};
mod sharing;
use serde_json::json;
pub use setup::{ChromeControlConfig, ChromeSetupView};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
pub use work::ChromeWorkView;

/// The always-enabled legacy network policy and its delivery generation. The number grows with every change in this
/// run of the app; it goes with each delivery (`generation`), so the app records an acknowledgement only for the
/// current change and a helper that knows it refuses a change older than the one it holds.
#[derive(Clone, Copy)]
struct LocalNetwork {
    on: bool,
    generation: u64,
}
pub(crate) struct ChromeControl {
    /// Headless fallback; a configured native browser owns the shared tracker instead.
    actions: std::sync::OnceLock<vw_preview::action_tracking::ActionTracker>,
    dir: Option<PathBuf>,
    switches: Mutex<store::Switches>,
    chrome_generation: AtomicU64,
    edge_generation: AtomicU64,
    firefox_generation: AtomicU64,
    switch_generation: AtomicU64,
    /// Legacy network-policy delivery state (see [`LocalNetwork`]).
    local_network: Mutex<LocalNetwork>,
    pub(crate) config: ChromeControlConfig,
    owner: owner::Owner,
    lifecycle: tokio::sync::Mutex<()>,
    generation: AtomicU64,
    /// Revokes Safari replies across an off/on switch cycle without invalidating Chrome work.
    safari_generation: AtomicU64,
    /// Registered installation diagnostics, independent of the helper connection.
    safari_copies: Mutex<Vec<String>>,
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
    local_documents: Mutex<local_documents::Documents>,
    click_refs: Mutex<approval::ClickRefs>,
    profiles: Mutex<profiles::Profiles>,
    /// Since when each browser has had no live paired profile (`profile_reason::observe`).
    offline_since: Mutex<std::collections::BTreeMap<&'static str, std::time::Instant>>,
    /// macOS: browser user data folders the profile-name search checked in this run (`browser_data`).
    name_folders: Mutex<browser_data::Checked>,
    bindings: Mutex<profile_bindings::Bindings>,
    binding_changes: tokio::sync::Mutex<()>,
    chat_profiles: Mutex<std::collections::HashMap<String, String>>,
    /// The profile the person picked for a chat on the profile card (`profile_selection::select_picked`), while it
    /// is still that chat's profile: a call naming another profile is told which one the user picked.
    picked_profiles: Mutex<std::collections::HashMap<String, String>>,
    /// The compatibility generation the running helper acknowledged (`None`: not yet, or a new helper connection). A
    /// change stays pending, and is sent again, until the helper acknowledges that very change.
    local_network_acked: Mutex<Option<u64>>,
    /// One compatibility delivery at a time, from reading the state to the helper's answer: a later delivery reads the
    /// state only after an earlier one was answered, so an older value can never reach the helper after a newer one.
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
    pub(crate) fn new(_vibe_home: &Path, config: ChromeControlConfig) -> Self {
        let dir = owner::state_dir(&config);
        let switches = store::Switches::load(dir.as_deref());
        let bindings = profile_bindings::Bindings::load(dir.as_deref());
        let profiles = profiles::Profiles::load(dir.as_deref());
        let chat_profiles = bindings.chats();
        let paired_once = dir.as_deref().is_some_and(store::paired_once);
        let edge_paired_once = dir
            .as_deref()
            .is_some_and(|dir| store::paired_once(&dir.join("edge")));
        Self {
            actions: std::sync::OnceLock::new(),
            dir,
            switches: Mutex::new(switches),
            chrome_generation: AtomicU64::new(0),
            edge_generation: AtomicU64::new(0),
            firefox_generation: AtomicU64::new(0),
            switch_generation: AtomicU64::new(0),
            local_network: Mutex::new(LocalNetwork {
                on: true,
                generation: 0,
            }),
            config,
            owner: owner::Owner::default(),
            lifecycle: tokio::sync::Mutex::new(()),
            generation: AtomicU64::new(0),
            safari_generation: AtomicU64::new(0),
            safari_copies: Mutex::new(Vec::new()),
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
            local_documents: Mutex::new(local_documents::Documents::default()),
            click_refs: Mutex::new(approval::ClickRefs::default()),
            profiles: Mutex::new(profiles),
            offline_since: Mutex::new(std::collections::BTreeMap::new()),
            name_folders: Mutex::new(browser_data::Checked::new()),
            bindings: Mutex::new(bindings),
            binding_changes: tokio::sync::Mutex::new(()),
            chat_profiles: Mutex::new(chat_profiles),
            picked_profiles: Mutex::new(std::collections::HashMap::new()),
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
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }
    /// Always enabled for compatibility with extensions that still use the legacy address policy.
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
    /// Refreshes the enabled compatibility policy; old callers cannot reinstate address restrictions.
    pub(crate) fn set_local_network(&self, _on: bool) {
        // Advance under one lock so acknowledgements bind the current delivery.
        let mut state = self
            .local_network
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *state = LocalNetwork {
            on: true,
            generation: state.generation + 1,
        };
    }
    /// A New-chat context window follows its draft to the newly started chat before the first turn.
    pub(crate) async fn adopt_tab_context(&self, old: &str, new: &str) -> Result<(), String> {
        profile_bindings::adopt(self, old, new)?;
        if !self.config.available() || self.owner.connected_profiles.snapshot().is_empty() {
            return Ok(());
        }
        let _ = self
            .owner
            .call(
                "chrome_status",
                serde_json::json!({"_adoptChat":{"old":old,"new":new}}),
            )
            .await;
        Ok(())
    }
    /// A profile is connected now: remembered for the setup's lifetime (`paired_once`).
    pub(super) fn mark_paired(&self) {
        installation::remember_directory(self, agent_browser::Browser::Chrome);
        if !self.paired_once.swap(true, Ordering::SeqCst)
            && let Some(dir) = self.dir()
            && let Err(reason) = store::save_paired_once(dir, true)
        {
            tracing::debug!(%reason, "the Chrome pairing record could not be saved");
        }
    }
    /// Chrome holds another owner: the earlier pairing no longer holds.
    pub(super) fn forget_paired(&self) {
        installation::forget_directory(self, agent_browser::Browser::Chrome);
        if self.paired_once.swap(false, Ordering::SeqCst)
            && let Some(dir) = self.dir()
        {
            let _ = store::save_paired_once(dir, false);
        }
    }
    pub(crate) fn reset(&self) {
        profiles::disconnected(self);
        self.connect_requested.store(false, Ordering::SeqCst);
        self.edge_connect_requested.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .disconnected();
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
    /// Release control after a completed turn while keeping its exact browser session available for follow-up.
    pub(crate) fn end_turn(&self, chat: &str) {
        self.release(chat, false);
    }
    /// A real person question retains the released session until its answer or the chat ends.
    pub(crate) fn handover(&self, chat: &str) {
        self.release(chat, true);
    }
    fn release(&self, chat: &str, awaiting_person: bool) {
        self.output_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear_chat(chat);
        let ids: Vec<_> = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, owner)| *owner == chat)
            .map(|(id, _)| id.clone())
            .collect();
        self.work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .release(chat);
        for id in ids {
            self.owner
                .control(json!({"action":"idle", "sessionId":id,"awaitingPerson":awaiting_person}));
        }
        self.stopped_by_person
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(chat.to_owned())
            .and_modify(|held| held.retain(|stop| stop.turn.is_some()));
    }
    /// The person stopped these browser sessions (owner, from the helper's status): each browser stays
    /// stopped for the turn, without reopening any old id. Claims one interrupt per turn,
    /// including when both browsers report Stop together.
    fn person_stopped(&self, stopped: &[person_stop::Stopped]) -> Vec<person_stop::Stopped> {
        let mut claimed = Vec::new();
        let mut held = self
            .stopped_by_person
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for stop in stopped {
            let person_stop::Stopped { chat, browser, .. } = stop;
            let browsers = held.entry(chat.clone()).or_default();
            if !browsers.iter().any(|held| held.turn == stop.turn) {
                claimed.push(stop.clone());
            }
            browsers.retain(|held| held.browser != *browser);
            browsers.push(stop.clone());
        }
        claimed
    }
    /// The person's Stop in Vibe Work ends this chat's browser authority.
    pub(crate) fn stop_chat(&self, chat: &str) {
        self.local_documents
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
        if deleted {
            self.local_documents
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(chat);
        }
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
        }
        if deleted {
            profile_bindings::forget(self, chat);
            self.picked_profiles
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
/// Before a `chrome_session` open: the person picks this chat's profile on a card when it is not decided
/// (`profile_seeding::ask_person`). The open's own arguments are unchanged; it then runs in the chat's binding.
pub(crate) async fn ask_profile(
    shared: &std::sync::Arc<crate::core::Shared>,
    chat: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    profile_seeding::ask_person(shared, chat, &mut args.clone()).await
}
/// Exact browser identity comparison, including the legacy raw Chrome UUID alias.
pub(crate) fn same_profile(left: &str, right: &str) -> bool {
    profile_ids::same(left, right)
}
pub(crate) fn end_chat(shared: &crate::core::Shared, chat: &str, deleted: bool) {
    shared.chrome_control.end_chat(chat, deleted);
    if deleted {
        profile_presence::prune(shared);
    }
    owner::publish(shared);
}
/// The person stopped the chat in Vibe Work: its Chrome work ends and is not reopened by an earlier id.
pub(crate) fn stop_chat(shared: &crate::core::Shared, chat: &str) {
    shared.chrome_control.stop_chat(chat);
    owner::publish(shared);
}
/// Publish availability after the question and all browsers' released state are visible.
pub(crate) fn publish_handover(shared: &crate::core::Shared) {
    owner::publish(shared);
}
/// A completed turn releases visual control and retains the session for follow-up.
pub(crate) fn end_turn(shared: &crate::core::Shared, chat: &str) {
    if shared.asks.question_hold(chat).is_some() {
        shared.chrome_control.handover(chat);
    } else {
        shared.chrome_control.end_turn(chat);
    }
    owner::publish(shared);
}
#[cfg(feature = "ts")]
mod declarations;
#[cfg(feature = "ts")]
pub(crate) use declarations::declarations;

#[cfg(test)]
mod lifecycle_tests;

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

#[cfg(all(test, feature = "your-edge"))]
mod browser_switch_tests;

mod switches;

#[cfg(any(target_os = "macos", all(test, unix)))]
mod safari_registration;

#[cfg(test)]
mod agent_outcome_tests;
#[cfg(test)]
mod sharing_tests;

#[cfg(test)]
mod page_media_tests;

#[cfg(test)]
mod local_file_tests;
mod page_capture;
pub(crate) mod page_files;
mod page_plan;
mod page_plan_catalog;
mod page_resources;
#[cfg(feature = "your-safari")]
mod safari_capture;

#[cfg(test)]
mod extension_state_tests;

mod extension_state;

#[cfg(test)]
mod page_capture_tests;

#[cfg(test)]
mod data_export_tests;

#[cfg(all(test, feature = "your-edge"))]
mod profile_binding_tests;

#[cfg(all(test, feature = "your-edge"))]
mod profile_agent_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_history_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_selection_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_settings_tests;

#[cfg(feature = "your-safari")]
mod safari_service;

#[cfg(test)]
mod browser_availability_text_tests;
