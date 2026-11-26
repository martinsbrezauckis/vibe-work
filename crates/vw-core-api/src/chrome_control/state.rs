//! The app's browser-control state: one value in the core's shared state (`Shared::chrome_control`), with the
//! per-chat release, Stop and end paths that every browser route shares. The module's facade is `super` (mod.rs).
use super::{
    ChromeControlConfig, agent_browser, agent_windows, approval, browser_data, external_video,
    frames, installation, local_documents, output_cache, owner, person_stop, probe,
    profile_bindings, profiles, selection, store, work,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) struct ChromeControl {
    /// Headless fallback; a configured native browser owns the shared tracker instead.
    pub(super) actions: std::sync::OnceLock<vw_preview::action_tracking::ActionTracker>,
    pub(super) dir: Option<PathBuf>,
    pub(super) switches: Mutex<store::Switches>,
    pub(super) chrome_generation: AtomicU64,
    pub(super) edge_generation: AtomicU64,
    pub(super) firefox_generation: AtomicU64,
    pub(super) switch_generation: AtomicU64,
    pub(crate) config: ChromeControlConfig,
    pub(super) owner: owner::Owner,
    pub(super) lifecycle: tokio::sync::Mutex<()>,
    /// Revokes Safari replies across an off/on switch cycle without invalidating Chrome work.
    pub(super) safari_generation: AtomicU64,
    /// Registered installation diagnostics, independent of the helper connection.
    pub(super) safari_copies: Mutex<Vec<String>>,
    /// A Connect Chrome click may open the unchanged extension's pairing page once.
    pub(super) connect_requested: AtomicBool,
    /// An Edge setup/store click may open Edge's own pairing page once.
    pub(super) edge_connect_requested: AtomicBool,
    /// A profile was connected with this setup at least once (kept in the state folder): a closed Chrome then
    /// reads "Connected when Chrome is open", never the pairing step again.
    pub(super) paired_once: AtomicBool,
    /// Edge's earlier live connection, separate from Chrome and never permission to act.
    pub(super) edge_paired_once: AtomicBool,
    /// Read-only look at Chrome's preferences (one probe at a time).
    pub(super) prober: probe::Prober,
    /// Edge's own preferences cache; a Chrome result must never answer an Edge probe.
    pub(super) edge_prober: probe::Prober,
    /// After Remove: the Chrome user data folders to check for an extension Chrome did not remove by itself.
    pub(super) left_in_chrome: Mutex<Vec<PathBuf>>,
    pub(super) removed_external: AtomicBool,
    pub(super) work: Mutex<work::Work>,
    /// What each helper session's agent control frame shows, and the person's applied pause clicks.
    /// Boxed for the same large-future limit as `Shared::paused_tools`.
    pub(super) frames: Box<Mutex<frames::Frames>>,
    pub(super) selections: Mutex<selection::Selections>,
    pub(super) tasks: Mutex<std::collections::HashMap<String, String>>,
    pub(super) output_cache: Mutex<output_cache::Cache>,
    pub(crate) external_video: external_video::Videos,
    pub(super) local_documents: Mutex<local_documents::Documents>,
    pub(super) click_refs: Mutex<approval::ClickRefs>,
    pub(super) profiles: Mutex<profiles::Profiles>,
    /// Since when each browser has had no live paired profile (`profile_reason::observe`).
    pub(super) offline_since: Mutex<std::collections::BTreeMap<&'static str, std::time::Instant>>,
    /// macOS: browser user data folders the profile-name search checked in this run (`browser_data`).
    pub(super) name_folders: Mutex<browser_data::Checked>,
    pub(super) bindings: Mutex<profile_bindings::Bindings>,
    pub(super) binding_changes: tokio::sync::Mutex<()>,
    pub(super) chat_profiles: Mutex<std::collections::HashMap<String, String>>,
    /// The profile the person picked for a chat on the profile card (`profile_selection::select_picked`), while it
    /// is still that chat's profile: a call naming another profile is told which one the user picked.
    pub(super) picked_profiles: Mutex<std::collections::HashMap<String, String>>,
    /// Last person Stop per browser and chat, bound to its turn and retained for racing replies.
    pub(super) stopped_by_person:
        Mutex<std::collections::HashMap<String, Vec<person_stop::Stopped>>>,
    /// Per chat: (profile, browser incarnation, window) of each tab its agent created (memory only,
    /// `agent_close::remember`). After a reconnect the extension hands the agent's windows to the person and the next
    /// session opens a new one; `close_browser` names these windows too, and the helper still closes only this chat's
    /// own tabs there. A window is nominated only in the browser incarnation it was opened in. Bounded in
    /// chats and per chat; a deleted chat's entry goes with the chat (`end_chat`).
    pub(super) agent_windows: Mutex<agent_windows::AgentWindows>,
}
impl ChromeControl {
    pub(crate) fn new(_vibe_home: &Path, config: ChromeControlConfig) -> Self {
        let dir = owner::state_dir(&config);
        let switches = store::Switches::load(dir.as_deref());
        let bindings = profile_bindings::Bindings::load(dir.as_deref());
        let profiles = profiles::Profiles::load(dir.as_deref());
        let chat_profiles = bindings.chats();
        let mut agent_windows = super::agent_windows::AgentWindows::default();
        agent_windows.seed_dropped(bindings.unclosed_windows());
        agent_windows.seed_warned(bindings.warned_windows());
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
            config,
            owner: owner::Owner::default(),
            lifecycle: tokio::sync::Mutex::new(()),
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
            frames: Box::default(),
            selections: Mutex::new(selection::Selections::default()),
            tasks: Mutex::new(std::collections::HashMap::new()),
            output_cache: Mutex::new(output_cache::Cache::default()),
            external_video: external_video::Videos::default(),
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
            agent_windows: Mutex::new(agent_windows),
        }
    }
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
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
    pub(super) fn release(&self, chat: &str, awaiting_person: bool) {
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
    pub(super) fn person_stopped(
        &self,
        stopped: &[person_stop::Stopped],
    ) -> Vec<person_stop::Stopped> {
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
        self.external_video.cancel_chat(chat);
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
            self.agent_windows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove_chat(chat);
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
