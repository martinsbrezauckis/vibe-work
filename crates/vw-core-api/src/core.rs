//! `Core`: both clients' shared facade for commands, events, sessions, supervisor and method gate.
//! Depends on vw-supervisor, vw-session, vw-gate, pump, transport, confirmer, subscription and view.
//! Called by the app's Tauri bridge, vw-debug-api and tests. Invariants:
//! - One gate per core: tokens, callbacks, picked folders and model aliases survive server restarts.
//!   Transport routes requests to the session's channel or the current server's lobby (`channels`).
//! - The pump subscribes to the first connection before `start` returns, so no session notification is missed.
//! - `Core` is an Arc handle. Drop kills the supervisor/terminal trees; `shutdown` stops terminals first.

#[path = "core_start.rs"]
mod start;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::access::Access;
use crate::channels::Channels;
use crate::confirmer::DynConfirmer;
pub use crate::core_config::CoreConfig;
pub use crate::error::CoreStartError;
use crate::event::CoreEvent;
use crate::names::Names;
use crate::subscription::CoreSubscription;
use crate::transport::SupervisedTransport;
use crate::view::{
    ChannelView, ChannelsReport, CoreStatus, DebugApiFlag, ServerReport, ServerStatus, duration_ms,
};

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::info;
use vw_gate::{Gate, GateError};
use vw_session::{Applied, SessionStore};
use vw_supervisor::Supervisor;
use vw_terminal::TerminalManager;
/// State shared by every handle, the pump and the server-request handler.
pub(crate) struct Shared {
    pub(crate) browser_settings: crate::browser_settings::Settings,
    pub(crate) browser_dialogs: Arc<crate::browser_dialogs::Dialogs>,
    pub(crate) feedback: crate::feedback::Feedback,
    pub(crate) deletes: Arc<crate::session_delete::Deletes>,
    pub(crate) guard_asks: crate::ask::guards::Calls,
    /// The question for the person's Git name and email, when the agent's commit has none.
    pub(crate) identity_asks: crate::git_identity_ask::IdentityAsks,
    pub(crate) asks: Arc<crate::ask::Service>,
    /// "Allow for this session" answers Vibe Work keeps for Vibe's approvals; Vibe gets `approve` only.
    pub(crate) ask_grants: crate::ask::grants::Grants,
    /// Install offers the window showed before a one-click install.
    pub(crate) install_offers: crate::install_offers::Offers,
    pub(crate) guard_refusals: crate::guard_refusals::Refusals,
    /// Edit and write previews of open approval cards (`edit_preview.rs`).
    pub(crate) edit_previews: crate::edit_preview::Previews,
    pub(crate) protected_changes: crate::protected_changes::Hub,
    pub(crate) github_reads: Arc<crate::github_reads::Reader>,
    pub(crate) keep_awake: crate::keep_awake::AwakeState,
    /// The New chat screen's owner of terminals, browser and port lease before the first message (`new_chat`).
    pub(crate) new_chat: crate::new_chat::Draft,
    /// Orders skills changes with admitting new work so a busy check cannot race a turn start.
    pub(crate) skills_lock: tokio::sync::Mutex<()>,
    /// One `vibeRules` removal at a time: each writes back the list it read, so two at once would put back the
    /// pattern the other removed (`vibe_rules`).
    pub(crate) vibe_rules_lock: tokio::sync::Mutex<()>,
    pub(crate) network_notice: std::sync::atomic::AtomicBool,
    pub(crate) secret_scan: crate::secret_scan::ScanState,
    /// User-controlled agent access pause; separate from cryptographic vault lock.
    pub(crate) agent_access_locked: std::sync::atomic::AtomicBool,
    /// Whether the Debug API drives the window right now (the host's probe, [`Core::set_window_drive`]); unset
    /// means never (`command_origin::debug_api_acts`).
    pub(crate) window_drive: std::sync::OnceLock<crate::command_origin::WindowDrive>,
    pub(crate) supervisor: Arc<Supervisor>,
    pub(crate) store: SessionStore,
    pub(crate) history_loading: Arc<crate::history_loading::HistoryLoading>,
    /// Vault project authority captured before session opening, never refreshed from agent-writable metadata.
    pub(crate) entry_projects: crate::entry_projects::EntryProjects,
    /// Titles and folders seen in listings (delete dialog text, reveal-folder check).
    pub(crate) names: Arc<Names>,
    /// Which channel carries which session (shared with the gate's transport).
    pub(crate) channels: Arc<Channels>,
    /// The profile each session runs with, restored on every resume (`access.rs`).
    pub(crate) access: Access,
    pub(crate) model_choices: crate::model_choices::Choices,
    pub(crate) plugin_refresh: crate::plugin_refresh::Refresh,
    pub(crate) interrupted_send: crate::interrupted_send::State,
    pub(crate) model_requests: Option<Arc<dyn vw_supervisor::ModelRequests>>,
    /// Per-chat Stop generation: a continuation scheduled before a Stop is never sent (`stops.rs`).
    pub(crate) stops: crate::stops::Stops,
    pub(crate) queue_returns: crate::queue_return::Returns,
    /// Sessions Vibe could not reopen after a restart (`stuck.rs`).
    pub(crate) stuck: crate::stuck::StuckSessions,
    /// `session/read` resyncs the pump started (status report; asserts none while sessions run).
    pub(crate) resync_reads: AtomicU64,
    pub(crate) gate: Gate<SupervisedTransport, DynConfirmer>,
    pub(crate) events: broadcast::Sender<CoreEvent>,
    pub(crate) config: CoreConfig,
    pub(crate) app_context: crate::app_context::Context,
    /// Store updates made by commands, for the pump's bookkeeping (callbacks, resyncs).
    pub(crate) loads: mpsc::Sender<Applied>,
    /// Process and port monitor (`processes.rs`).
    pub(crate) procmon: vw_procmon::ProcMon,
    pub(crate) vibe_processes: crate::vibe_processes::VibeProcesses,
    /// What the startup sweep found and did.
    pub(crate) sweep: Mutex<Option<vw_procmon::SweepReport>>,
    /// The Changes tab's trackers and refresh task (`changes.rs`).
    pub(crate) changes: crate::changes::Hub,
    /// Session-end state: deferred archive teardown and titles of deleted sessions with surviving processes.
    pub(crate) ends: crate::session_end::Ends,
    procmon_tick: Mutex<Option<JoinHandle<()>>>,
    /// The server pid last handed to the monitor (0: none), so the tick records a restarted server.
    pub(crate) ledger_server_pid: std::sync::atomic::AtomicU32,
    /// The session terminals.
    pub(crate) terminals: TerminalManager,
    /// Where the instruction files live (`instructions.rs`).
    pub(crate) instructions: crate::instructions::InstructionsPaths,
    /// Documents added and not yet in a message (`documents.rs`).
    pub(crate) documents: vw_documents::DocumentStore,
    /// Saved attachment grants and read results retained for each chat.
    pub(crate) saved_documents: crate::saved_documents::SavedDocuments,
    /// Running dictations (`dictation.rs`).
    pub(crate) dictations: Arc<crate::dictation::Dictations>,
    pub(crate) voice_prompts: crate::voice_prompts::Prompts,
    pub(crate) readings: crate::read_aloud::Pending,
    /// `panel_read` requests waiting for the window's answer.
    pub(crate) panel_reads: crate::panel_read::Pending,
    /// The running server sign-in and the catalog-loaded flag (`connectors.rs`).
    pub(crate) connectors: crate::connectors::ConnectorsState,
    /// The marketplace's catalog, downloader and running install (`market.rs`).
    pub(crate) market: crate::market::MarketState,
    pub(crate) plugin_approvals: Arc<crate::plugin_approvals::PluginApprovals>,
    /// The user's Chrome Control permissions, cards and setup (`chrome_control`).
    pub(crate) chrome_control: crate::chrome_control::ChromeControl,
    /// The session-scoped agent tools (`agent_tools`), when configured and started.
    pub(crate) agent_tools: Option<Arc<crate::agent_tools::AgentTools>>,
    /// The previews (`previews`), when the app gave an engine.
    pub(crate) preview: Option<Arc<crate::previews::PreviewHub>>,
    /// Project Pages (`pages`), when the app gave a viewer engine.
    pub(crate) pages: Option<Arc<crate::pages::PagesHub>>,
    /// The File tab (`files`): file handles and the files viewer.
    pub(crate) files: crate::files::FilesState,
    /// The `vw-plan` access mode's profile file (`plan_profile`).
    pub(crate) plan_profile: crate::plan_profile::PlanProfile,
    /// The user's project groups (`projects`, `projects_filing`).
    pub(crate) projects: crate::projects_filing::ProjectsState,
    /// Native-dialog confirmer, shared with the gate and core's own requests (Debug API terminal input).
    pub(crate) confirmer: DynConfirmer,
    /// Dialog sentences the core wrote for one exact gated call (`dialog_texts.rs`).
    pub(crate) dialog_texts: Arc<crate::dialog_texts::DialogTexts>,
    /// Session copies (`worktrees.rs`): record folder and the one-at-a-time lock.
    pub(crate) worktrees: crate::worktrees::WorktreesState,
    /// Temporary sessions (`temporary.rs`).
    pub(crate) temporary: Arc<crate::temporary::TemporarySessions>,
    /// The message index of the search (`search_index.rs`).
    pub(crate) search: crate::search_index::MessageIndex,
    /// Sessions another process holds (`session_holder.rs`), and the task that checks the viewed ones.
    pub(crate) holders: crate::session_holder::Holders,
    pub(crate) child_listings: crate::session_listing::ChildListings,
    /// App session claims shared by every command and channel.
    pub(crate) leases: Arc<crate::session_lease::Leases>,
    /// The saved-project repository tree watches (`git_browser_watch.rs`).
    pub(crate) git_browser: crate::git_browser_watch::BrowserWatches,
    /// The outer git loop (`git_cmd.rs`): data folders, the one-at-a-time lock, PR cards.
    pub(crate) git: crate::git_cmd::GitHub,
    /// The secret broker's use log (`secret_uses.rs`).
    pub(crate) secret_uses: crate::secret_uses::SecretUses,
    /// Libraries' scopes, the image switch and image counters (`work.rs`).
    pub(crate) work: crate::work::WorkState,
    started: Instant,
    debug_api: watch::Sender<DebugApiFlag>,
    pump: Mutex<Option<JoinHandle<()>>>,
    /// Forwards key-service events as core events (owned, aborted with the pump).
    key_events: Mutex<Option<JoinHandle<()>>>,
    /// Re-emits the Vibe install's view changes.
    vibe_events: Mutex<Option<JoinHandle<()>>>,
    /// Re-reads the vault values chat titles are compared with when the vault opens or changes (`title_guard`).
    title_events: Mutex<Option<JoinHandle<()>>>,
    /// Withholds vault-value parts in every chat title and preview from Vibe (`title_guard`).
    pub(crate) titles: crate::title_guard::TitleGuard,
}

impl Shared {
    /// Publishes a core event to every subscriber (none listening is fine).
    pub(crate) fn emit(&self, event: CoreEvent) {
        if matches!(
            event,
            CoreEvent::CallbackOpened { .. } | CoreEvent::CallbackAnswered { .. }
        ) {
            crate::keep_awake::reconcile(self);
        }
        let _ = self.events.send(event);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.secret_scan.stop();
        if let Some(tick) = self
            .procmon_tick
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            tick.abort();
        }
        for slot in [
            &self.pump,
            &self.key_events,
            &self.vibe_events,
            &self.title_events,
        ] {
            if let Some(task) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                task.abort();
            }
        }
    }
}

/// Handle to the running core. Clone it freely.
#[derive(Clone)]
pub struct Core {
    pub(crate) shared: Arc<Shared>,
}

impl Core {
    /// The agent browser's network guard, when this run has one: the host registers the Debug API's
    /// port with it (Vibe Work's own ports are never reachable from a browser page).
    #[must_use]
    pub fn network_guard(&self) -> Option<vw_netguard::NetGuard> {
        self.shared
            .preview
            .as_ref()
            .and_then(|hub| hub.guard.clone())
    }

    /// Subscribes to every session (`None`) or one session: the first items are snapshots, then
    /// ordered changes and core events. A slow subscriber gets `lagged` and fresh snapshots.
    #[must_use]
    pub fn subscribe(&self, session: Option<String>) -> CoreSubscription {
        CoreSubscription::new(&self.shared, session)
    }

    /// Records a folder the user picked in a native dialog (the only folders that may be
    /// trusted). Not reachable from the webview; the Debug API has its own `pickFolder` command
    /// (it acts as the user).
    ///
    /// # Errors
    /// The gate's error when the path is not an existing directory.
    pub fn register_picked_folder(&self, path: &Path) -> Result<(), GateError> {
        self.shared.gate.register_picked_folder(path)
    }

    /// Installs the host's answer to "is a Debug API `ui/*` request being served right now?" (once per core; a later
    /// call is ignored). While it says yes, window commands keep the window's authority but are attributed to the
    /// Debug API and get its vault restriction (`command_origin::debug_api_acts`); it never grants anything.
    pub fn set_window_drive(&self, probe: impl Fn() -> bool + Send + Sync + 'static) {
        let _ = self.shared.window_drive.set(Box::new(probe));
    }

    /// True when `path` counts as picked in this run (a native pick, or the Debug API's `pickFolder`; a folder
    /// inside one counts). For tests and the host.
    #[must_use]
    pub fn is_picked_folder(&self, path: &Path) -> bool {
        self.shared.gate.is_picked_folder(path)
    }

    /// True when a session (listed or loaded) works in exactly `path` or has it as a workspace root.
    /// Folder reveal uses [`Self::reveal_folder_target`] to admit contained subfolders too.
    #[must_use]
    pub fn is_session_folder(&self, path: &str) -> bool {
        self.shared
            .names
            .is_session_folder(&self.shared.store, path)
    }

    /// Sets the "Debug API on" flag (called by vw-debug-api when it starts or stops listening)
    /// and tells subscribers.
    pub fn set_debug_api_flag(&self, flag: DebugApiFlag) {
        self.shared.debug_api.send_replace(flag.clone());
        self.shared.emit(CoreEvent::DebugApi { flag });
    }

    /// The current "Debug API on" flag, for the UI indicator.
    #[must_use]
    pub fn debug_api_flag(&self) -> DebugApiFlag {
        self.shared.debug_api.borrow().clone()
    }

    /// Supervisor state, versions, uptime, flag and loaded sessions.
    #[must_use]
    pub fn status(&self) -> CoreStatus {
        let shared = &self.shared;
        let state = shared.supervisor.state().borrow().clone();
        let status = ServerStatus::from(&state);
        let vibe_version = match &status {
            ServerStatus::Running { server_version, .. } => Some(server_version.clone()),
            _ => None,
        };
        let stderr_tail = match &state {
            vw_supervisor::SupervisorState::Restarting { last_death, .. } => {
                last_death.stderr_tail.clone()
            }
            vw_supervisor::SupervisorState::Failed {
                last_death: Some(death),
                ..
            } => death.stderr_tail.clone(),
            _ => shared
                .supervisor
                .current()
                .map(|c| c.stderr_tail(shared.config.stderr_lines))
                .unwrap_or_default(),
        };
        CoreStatus {
            app_version: shared
                .config
                .app_version
                .clone()
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
            vibe_version,
            uptime_ms: duration_ms(shared.started.elapsed()),
            server: ServerReport {
                status,
                pid: shared.supervisor.server_pid(),
                stderr_tail,
            },
            debug_api: self.debug_api_flag(),
            loaded_sessions: shared
                .store
                .views()
                .into_iter()
                .map(|v| v.session_id)
                .collect(),
            channels: ChannelsReport {
                open: shared
                    .channels
                    .open_channels()
                    .into_iter()
                    .map(|c| ChannelView {
                        session_id: c.session_id,
                        channel_id: c.channel_id,
                        idle_ms: duration_ms(c.idle),
                    })
                    .collect(),
                foreign_dropped: shared.channels.foreign.load(Ordering::Relaxed),
                stuck: shared.stuck.list(),
                resync_reads: shared.resync_reads.load(Ordering::Relaxed),
            },
            rate_limits: shared
                .config
                .keys
                .as_ref()
                .and_then(|keys| keys.rate_limits())
                .map(Into::into),
        }
    }
    /// Stops the pump and owned processes, returns leases, then shuts the server down cleanly.
    /// Agent processes stay in the ledger; stdin closes before the grace period and kill.
    pub async fn shutdown(&self) {
        if let Some(requests) = &self.shared.model_requests {
            requests.shutdown();
        }
        self.shared.history_loading.stop_all();
        self.shared.asks.cancel_session(None, "Vibe Work closed");
        self.shared.changes.shutdown().await;
        self.shared.protected_changes.shutdown();
        crate::chrome_control::stop_owner(&self.shared).await;
        let reader = self.shared.github_reads.clone();
        let _ = tokio::task::spawn_blocking(move || reader.shutdown()).await;
        self.shared.keep_awake.shutdown().await;
        self.shared.secret_scan.shutdown().await;
        self.shared.git_browser.stop_all();
        if let Some(clipboard) = &self.shared.config.clipboard {
            clipboard.shutdown().await;
        }
        let tick = self
            .shared
            .procmon_tick
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(tick) = tick {
            tick.abort();
        }
        crate::connectors::abort_sign_in(&self.shared);
        self.shared.holders.stop_poll();
        crate::market::abort_install(&self.shared);
        // Terminals first: every shell tree of every session ends with the app.
        let _ = self.shared.terminals.close_all().await;
        let monitor = self.shared.procmon.clone();
        let sessions = crate::processes::contexts(&self.shared.store);
        if let Ok(reports) = tokio::task::spawn_blocking(move || monitor.shutdown(&sessions)).await
        {
            info!(
                sessions = reports.len(),
                "process monitor: owned processes stopped"
            );
        }
        for slot in [
            &self.shared.pump,
            &self.shared.key_events,
            &self.shared.vibe_events,
            &self.shared.title_events,
        ] {
            let task = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(task) = task {
                task.abort();
            }
        }
        if let Some(tools) = &self.shared.agent_tools {
            tools.shutdown();
        }
        // Previews close with the app (their website data stays for the next run).
        crate::previews::shutdown(&self.shared).await;
        // The page viewer closes and the page servers stop (comments and settings stay in app data).
        crate::pages::shutdown(&self.shared).await;
        crate::files::shutdown(&self.shared).await;
        // temporary sessions are deleted while Vibe still runs (bounded per session).
        self.delete_temporary_all(false).await;
        if let Some(keys) = &self.shared.config.keys {
            keys.begin_shutdown().await;
        }
        let exit = self.shared.supervisor.stop().await;
        self.shared.vibe_processes.shutdown();
        self.shared.leases.release_all();
        // The stopped server leaves the ledger (agent processes stay recorded for the next start).
        crate::processes::record_server(&self.shared).await;
        self.shared.emit(CoreEvent::Server {
            status: ServerStatus::Stopped {
                exit_code: exit.and_then(|e| e.code),
            },
        });
        info!(?exit, "core stopped");
    }
}
