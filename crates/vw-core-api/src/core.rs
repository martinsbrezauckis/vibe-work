//! `Core`: the one object both clients talk to. It owns the supervisor, the session store and the
//! method gate, runs the notification pump, and exposes `execute` (commands), `subscribe`
//! (events) and the "Debug API on" flag.
//!
//! Depends on: vw-supervisor (process), vw-session (store), vw-gate (policy), `pump`, `transport`,
//! `confirmer`, `subscription`, `view`. Called by: the app (Tauri bridge), vw-debug-api, tests.
//! Invariants:
//! - One gate per core: tokens, open callbacks, picked folders and model aliases survive server
//!   restarts (the transport routes each request to the session's channel or the lobby of the
//!   supervisor's current server, `channels`).
//! - The pump is subscribed to the first connection before `start` returns, so no notification of
//!   a session started through this core is missed.
//! - `Core` is a cheap handle (`Arc` inside). When the last handle drops, the supervisor drops and
//!   kills the server tree, and the terminal manager drops and kills every terminal tree;
//!   `shutdown` does the same cleanly (terminals first).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Instant;

use crate::access::Access;
use crate::channels::Channels;
use crate::confirmer::DynConfirmer;
pub use crate::core_config::CoreConfig;
pub use crate::error::CoreStartError;
use crate::event::CoreEvent;
use crate::names::Names;
use crate::pump;
use crate::subscription::CoreSubscription;
use crate::transport::SupervisedTransport;
use crate::view::{
    ChannelView, ChannelsReport, CoreStatus, DebugApiFlag, ServerReport, ServerStatus, duration_ms,
};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::info;
use vw_gate::{Confirmer, Gate, GateError};
use vw_session::{Applied, SessionStore};
use vw_supervisor::{SecretProvider, Supervisor, SupervisorConfig, SupervisorError};
use vw_terminal::TerminalManager;
/// State shared by every handle, the pump and the server-request handler.
pub(crate) struct Shared {
    pub(crate) deletes: Arc<crate::session_delete::Deletes>,
    pub(crate) guard_asks: crate::ask::guards::Calls,
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

/// The supervisor: started with its server, or waiting with no server (nothing spawned) when Vibe is
/// not installed yet; the install's `on_ready` hook launches it later ([`connect_vibe_install`]).
async fn start_supervisor(
    mut supervisor: SupervisorConfig,
    secrets: Arc<dyn SecretProvider>,
    config: &CoreConfig,
) -> Result<Arc<Supervisor>, SupervisorError> {
    crate::mcp_keys::prepare_config(&supervisor.vibe_home, config.keys.as_ref()); // Use this run's port.
    if !config.start_vibe {
        // Local startup recovery and plugin admission still belong to the core even when a test needs no server.
        if let Some(check) = &supervisor.before_launch {
            check(&supervisor.program).map_err(SupervisorError::BeforeLaunch)?;
        }
        return Ok(Arc::new(Supervisor::start_pending(
            supervisor,
            secrets,
            "This core runs without a Vibe server".to_owned(),
        )));
    }
    if let Some(installer) = config.vibe_install.as_ref() {
        installer.check_saved_existing().await;
        if let Some(server) = installer.server() {
            supervisor.program = server;
        }
    }
    Ok(match config.vibe_install.as_ref() {
        Some(installer) if installer.server().is_none() => Arc::new(Supervisor::start_pending(
            supervisor,
            secrets,
            "Vibe is not installed yet: install it on the setup screen".to_owned(),
        )),
        _ => Arc::new(Supervisor::start(supervisor, secrets).await?),
    })
}

/// lets the installer start the first server when Vibe is installed (or an existing install is
/// chosen), and re-emits its view changes as `vibeInstall` events until the core goes away.
fn connect_vibe_install(shared: &Arc<Shared>) {
    let Some(installer) = shared.config.vibe_install.clone() else {
        return;
    };
    let weak = Arc::downgrade(shared);
    installer.set_on_ready(Arc::new(move |server| {
        let weak = weak.clone();
        Box::pin(async move {
            let Some(shared) = weak.upgrade() else {
                return Err("Vibe Work is closing".to_owned());
            };
            match shared.supervisor.launch_installed(&server).await {
                Ok(()) => {
                    crate::processes::record_server(&shared).await;
                    Ok(())
                }
                // A supervisor started with a server (the normal case) has nothing to launch.
                Err(vw_supervisor::SupervisorError::Unsupported(_))
                    if shared.supervisor.current().is_some() =>
                {
                    Ok(())
                }
                Err(error) => Err(format!("Vibe did not start: {error}")),
            }
        })
    }));
    let mut views = installer.subscribe();
    if let Err(error) = installer.upgrade_previous_private() {
        tracing::warn!(%error, "could not prepare the updated app's Vibe install");
    }
    let weak = Arc::downgrade(shared);
    let task = tokio::spawn(async move {
        while views.changed().await.is_ok() {
            let view = views.borrow_and_update().clone();
            let Some(shared) = weak.upgrade() else {
                return;
            };
            shared.emit(CoreEvent::VibeInstall { view });
        }
    });
    *shared
        .vibe_events
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(task);
}

/// Starts the owned task that re-emits the key service's events (when the core has a key service).
fn spawn_key_forwarder(shared: &Arc<Shared>) {
    if let Some(keys) = &shared.config.keys {
        let forwarder = tokio::spawn(crate::key_events::forward(
            Arc::downgrade(shared),
            keys.subscribe(),
        ));
        *shared
            .key_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(forwarder);
    }
}

/// Starts the core's owned background tasks: the agent tools' listener, the preview event forwarder, the
/// Changes refresh and the process monitor's tick (each aborted on shutdown or when the core drops). First it
/// registers the credentials of Vibe Work's own Vibe config as stored secrets, before a session can
/// echo them, and installs the MCP forwarder's resolver.
fn spawn_owned_tasks(shared: &Arc<Shared>) {
    crate::mcp_keys::at_start(shared);
    crate::agent_tools::start_server(shared);
    crate::previews::start_events(shared);
    crate::pages::start_events(shared);
    crate::files::start_events(shared);
    crate::plan_profile::start(shared);
    shared.changes.spawn(shared);
    *shared
        .procmon_tick
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(crate::processes::spawn_tick(shared));
    // which viewed sessions another process holds.
    crate::session_holder::start_poll(shared);
}

/// Handle to the running core. Clone it freely.
#[derive(Clone)]
pub struct Core {
    pub(crate) shared: Arc<Shared>,
}

/// Makes the core the `callback/call` handler of `supervisor` (any existing one is replaced: the core owns
/// callbacks); the returned slot is filled with the core once it exists.
fn own_callbacks(supervisor: &mut SupervisorConfig) -> Arc<OnceLock<Weak<Shared>>> {
    let slot: Arc<OnceLock<Weak<Shared>>> = Arc::new(OnceLock::new());
    supervisor
        .handlers
        .retain(|(method, _)| method != "callback/call");
    supervisor.handlers.push((
        "callback/call".to_owned(),
        Arc::new(pump::callback_handler(slot.clone())),
    ));
    slot
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

    /// Starts the server under supervision, builds the store and the gate, and starts the pump.
    ///
    /// `confirmer` shows the native dialogs for calls that need a confirmation; it must fail
    /// closed. The `callback/call` handler is added to `supervisor` (any existing one for that
    /// method is replaced: the core owns callbacks).
    ///
    /// The process monitor is created first; with a ledger folder configured, the startup sweep then stops
    /// what an earlier crashed instance left of its own (`owned`) processes and re-attaches the agent's
    /// (`agent`) ones (`Core::startup_sweep`).
    ///
    /// # Errors
    /// [`CoreStartError::Supervisor`] when the first server start fails; [`CoreStartError::Procmon`] when
    /// the ledger folder cannot be prepared; [`CoreStartError::Marketplace`] when an interrupted plugin update
    /// cannot be recovered.
    #[allow(clippy::too_many_lines)]
    pub async fn start(
        mut supervisor: SupervisorConfig,
        secrets: Arc<dyn SecretProvider>,
        confirmer: impl Confirmer + 'static,
        config: CoreConfig,
    ) -> Result<Self, CoreStartError> {
        let mut config = crate::secrets::configure_clipboard(config)?;
        crate::agent_tools::secret_env::inherit_path(&supervisor, &mut config)?;
        let github_reads = crate::github_reads::Reader::new(&config, &supervisor.vibe_home);
        crate::github_reads::prepare(&github_reads, &mut supervisor, &mut config);
        let procmon = vw_procmon::ProcMon::new(config.procmon.clone())?;
        let sweep = crate::processes::startup_sweep(&procmon).await;
        let instructions = crate::instructions::InstructionsPaths {
            own_home: supervisor.vibe_home.clone(),
            cli_home: config.instructions.cli_home.clone(),
        };
        let leases = Arc::new(
            crate::session_lease::Leases::new(
                &instructions.own_home,
                config.sessions_dir.as_deref(),
            )
            .map_err(|error| CoreStartError::SessionOwnership(error.to_string()))?,
        );
        vw_market::Store::new(&instructions.own_home).recover_updates()?;
        let plugin_approvals = Arc::new(
            crate::plugin_approvals::PluginApprovals::configured(&supervisor, &config)
                .map_err(|e| CoreStartError::PluginApprovals(e.to_string()))?,
        );
        let approvals = plugin_approvals.clone();
        supervisor.before_launch = Some(Arc::new(move |program| {
            let _held = approvals.mutations.try_lock().map_err(|e| e.to_string())?;
            approvals.before_launch(program).map_err(|e| e.to_string())
        }));
        let plugin_roots = crate::market_discovery::builtin_roots(&supervisor);
        let chrome_control = crate::chrome_control::ChromeControl::new(
            &supervisor.vibe_home,
            config.chrome_control.clone(),
        );
        let deletes = Arc::new(crate::session_delete::Deletes::configure(
            &mut supervisor,
            config.sessions_dir.as_deref(),
        ));
        let slot = own_callbacks(&mut supervisor);
        let app_context = crate::app_context::Context::new(&supervisor, &config);
        let supervisor = start_supervisor(supervisor, secrets, &config).await?;
        let first = supervisor.current().map(|conn| conn.subscribe());
        let (feeds, feeds_rx) = mpsc::unbounded_channel();
        let agent_tools = crate::agent_tools::open(
            config.agent_tools.clone(),
            &procmon,
            config.chrome_control.available(),
        );
        let channels = Arc::new(Channels::new(
            supervisor.clone(),
            config.connection_wait,
            feeds,
            agent_tools.clone(),
            leases.clone(),
            plugin_approvals.clone(),
        ));
        let transport = SupervisedTransport {
            channels: channels.clone(),
        };
        let store = SessionStore::new();
        let names = Arc::new(Names::default());
        let (events, _) = broadcast::channel(config.event_capacity.max(1));
        let asks = crate::ask::Service::new(config.ask_clock.clone(), events.clone());
        let confirmer = DynConfirmer::new(confirmer, names.clone(), store.clone(), asks.clone());
        let gate = Gate::new(transport, confirmer.clone(), config.gate.clone());
        let terminals = crate::terminals::manager(&config, &procmon, &events);
        let (loads, loads_rx) = mpsc::channel(256);
        let (preview, pages, files) = crate::core_viewers::open(&config);
        let plan_profile = crate::plan_profile::PlanProfile::new(instructions.own_home.clone());
        let shared = Arc::new(Shared {
            deletes,
            app_context,
            asks,
            ask_grants: crate::ask::grants::Grants::default(),
            chrome_control,
            install_offers: crate::install_offers::Offers::default(),
            guard_asks: crate::ask::guards::Calls::default(),
            guard_refusals: crate::guard_refusals::Refusals::default(),
            edit_previews: crate::edit_preview::Previews::default(),
            protected_changes: crate::protected_changes::Hub::of(&config),
            github_reads,
            keep_awake: crate::keep_awake::from_config(&config),
            new_chat: crate::new_chat::Draft::new(config.sessions_dir.as_deref()),
            skills_lock: tokio::sync::Mutex::new(()),
            queue_returns: crate::queue_return::Returns::default(),
            vibe_rules_lock: tokio::sync::Mutex::new(()),
            network_notice: std::sync::atomic::AtomicBool::new(config.network_settings_rejected),
            secret_scan: crate::secret_scan::ScanState::new(config.secret_scan_data.as_deref()),
            agent_access_locked: std::sync::atomic::AtomicBool::new(false),
            window_drive: std::sync::OnceLock::new(),
            supervisor,
            store,
            entry_projects: crate::entry_projects::EntryProjects::default(),
            names,
            channels,
            access: Access::load(config.sessions_dir.clone()),
            model_choices: crate::model_choices::Choices::load(config.sessions_dir.as_deref()),
            plugin_refresh: crate::plugin_refresh::Refresh::default(),
            stops: crate::stops::Stops::default(),
            stuck: crate::stuck::StuckSessions::default(),
            resync_reads: AtomicU64::new(0),
            gate,
            // Built before `events` and `config` move into the struct (fields evaluate in this order).
            dictations: crate::dictation::Dictations::new(events.clone(), config.speech.clone()),
            readings: crate::read_aloud::Pending::default(),
            panel_reads: crate::panel_read::Pending::default(),
            changes: crate::changes::Hub::from_config(&config),
            worktrees: crate::worktrees::WorktreesState::of(&config, &instructions),
            git_browser: crate::git_browser_watch::BrowserWatches::default(),
            git: crate::git_cmd::GitHub::new(config.changes_dir.as_deref()),
            projects: crate::projects_filing::ProjectsState::new(config.projects_dir.clone()),
            market: crate::market::MarketState::new(&config.market, plugin_roots),
            plugin_approvals,
            // Only metadata from this app run is retained for the Settings use log.
            secret_uses: crate::secret_uses::SecretUses::default(),
            work: crate::work::WorkState::open(&config.work),
            temporary: Arc::new(crate::temporary::TemporarySessions::load(
                config.sessions_dir.clone(),
            )),
            search: crate::search_index::MessageIndex::default(),
            holders: crate::session_holder::Holders::default(),
            child_listings: crate::session_listing::ChildListings::default(),
            leases,
            events,
            config,
            loads,
            procmon,
            sweep: Mutex::new(sweep),
            ends: crate::session_end::Ends::default(),
            procmon_tick: Mutex::new(None),
            ledger_server_pid: std::sync::atomic::AtomicU32::new(0),
            terminals,
            instructions,
            documents: vw_documents::DocumentStore::default(),
            saved_documents: crate::saved_documents::SavedDocuments::default(),
            connectors: crate::connectors::ConnectorsState::default(),
            agent_tools,
            preview,
            pages,
            files,
            plan_profile,
            dialog_texts: confirmer.texts(),
            confirmer,
            started: Instant::now(),
            debug_api: watch::Sender::new(DebugApiFlag {
                enabled: false,
                reason: "off".to_owned(),
            }),
            pump: Mutex::new(None),
            key_events: Mutex::new(None),
            vibe_events: Mutex::new(None),
            title_events: Mutex::new(None),
            titles: crate::title_guard::TitleGuard::default(),
        });
        let _ = slot.set(Arc::downgrade(&shared));
        // Before any notification is applied: an open vault's values withhold titles from the first listing on.
        crate::title_guard::refresh(&shared).await;
        let watcher = tokio::spawn(crate::title_guard::watch(
            Arc::downgrade(&shared),
            shared.events.subscribe(),
        ));
        *shared
            .title_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watcher);
        let task = tokio::spawn(pump::run(
            Arc::downgrade(&shared),
            first,
            loads_rx,
            feeds_rx,
        ));
        *shared.pump.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
        spawn_key_forwarder(&shared);
        connect_vibe_install(&shared);
        crate::processes::note_vibe_server(&shared);
        crate::processes::record_server(&shared).await;
        // Session-list clients cannot run until startup returns after both cleanup sweeps.
        spawn_owned_tasks(&shared);
        Box::pin(
            Core {
                shared: shared.clone(),
            }
            .delete_temporary_all(true),
        )
        .await;
        Box::pin(crate::session_delete::sweep(&Core {
            shared: shared.clone(),
        }))
        .await;
        crate::github_reads::watch(&shared);
        crate::keep_awake::publish(&shared);
        crate::secret_scan::start(&shared);
        crate::chrome_control::restore_at_start(&shared);
        crate::chrome_control::start_owner(&shared).await;
        info!("core started");
        Ok(Self { shared })
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

    /// True when a session (listed or loaded) works in `path` or has it as a workspace root. The
    /// host uses it to refuse "reveal in file manager" for any other path.
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
    /// The last resort when [`Core::shutdown`] could not finish at quit: SIGKILL to the Vibe server's process group
    /// and its crash watchdog, synchronously ([`vw_supervisor::Supervisor::kill_now`]). Returns whether a server was
    /// running.
    #[must_use]
    pub fn stop_server_now(&self) -> bool {
        self.shared.supervisor.kill_now()
    }

    /// Stops the pump and owned processes, returns leases, then shuts the server down cleanly.
    /// Agent processes stay in the ledger; stdin closes before the grace period and kill.
    pub async fn shutdown(&self) {
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
        let exit = self.shared.supervisor.stop().await;
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
