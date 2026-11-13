//! Startup construction and owned-task wiring for `Core`.
//! Depends on the supervisor, shared core state and service owners; called only by `Core::start`.
//! The initial subscription precedes service work, and recovery sweeps finish before clients receive the core.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Instant;

use tokio::sync::{broadcast, mpsc, watch};
use tracing::info;
use vw_gate::{Confirmer, Gate};
use vw_session::SessionStore;
use vw_supervisor::{SecretProvider, Supervisor, SupervisorConfig, SupervisorError};

use super::{Core, CoreConfig, CoreStartError, Shared};
use crate::access::Access;
use crate::channels::Channels;
use crate::confirmer::DynConfirmer;
use crate::event::CoreEvent;
use crate::names::Names;
use crate::pump;
use crate::transport::SupervisedTransport;
use crate::view::DebugApiFlag;

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
        Some(installer) => {
            // A first start that fails (a cold start over its time limit, a broken install) leaves the core running
            // without a server: the setup screen says Vibe did not start, and its Try again starts it again
            // (`VibeInstaller::retry_start`) without restarting the app. A refused startup check stays fatal.
            let program = supervisor.program.clone();
            let pending =
                Supervisor::start_pending(supervisor, secrets, "Vibe is starting".to_owned());
            match pending.launch_installed(&program).await {
                Ok(()) => {}
                Err(error @ SupervisorError::BeforeLaunch(_)) => return Err(error),
                Err(error) => {
                    tracing::warn!(%error, "Vibe did not start; the setup screen offers Try again");
                    installer.start_failed(format!("Vibe did not start: {error}"));
                }
            }
            Arc::new(pending)
        }
        None => Arc::new(Supervisor::start(supervisor, secrets).await?),
    })
}

/// lets the installer start the first server when Vibe is installed (or an existing install is
/// chosen), and re-emits its view changes as `vibeInstall` events until the core goes away (and a `chromeControl`
/// event when the accepted manifest's Chrome store state changed).
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
    // A check that changes the Chrome store state also refreshes Your Chrome (no restart).
    let mut extension = installer.extension_store();
    let task = tokio::spawn(async move {
        while views.changed().await.is_ok() {
            let view = views.borrow_and_update().clone();
            let Some(shared) = weak.upgrade() else {
                return;
            };
            shared.emit(CoreEvent::VibeInstall { view });
            let now = shared
                .config
                .vibe_install
                .as_ref()
                .and_then(vw_vibe_install::VibeInstaller::extension_store);
            if now != extension {
                extension = now;
                shared.emit(CoreEvent::ChromeControl {
                    view: crate::chrome_control::commands::view(&shared),
                });
            }
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

/// Wires the core's owned services and starts their background work (stopped on shutdown or core drop).
/// Registers Vibe config credentials before a session can echo them, then starts agent tools, preview events,
/// browser popups, pages, files, plan profiles, Changes refresh, process monitoring and session-holder polling.
fn spawn_owned_tasks(shared: &Arc<Shared>) {
    crate::mcp_keys::at_start(shared);
    crate::agent_tools::start_server(shared);
    crate::previews::start_events(shared);
    crate::browser_popups::install(shared);
    crate::pages::start_events(shared);
    crate::files::start_events(shared);
    crate::plan_profile::start(shared);
    shared.changes.spawn(shared);
    *shared
        .procmon_tick
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(crate::processes::spawn_tick(shared));
    crate::session_holder::start_poll(shared);
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

/// Services prepared in startup order, then moved into the shared state without spawning its tasks.
struct Startup {
    model_requests: Option<Arc<dyn vw_supervisor::ModelRequests>>,
    vibe_processes: crate::vibe_processes::VibeProcesses,
    deletes: Arc<crate::session_delete::Deletes>,
    app_context: crate::app_context::Context,
    asks: Arc<crate::ask::Service>,
    chrome_control: crate::chrome_control::ChromeControl,
    github_reads: Arc<crate::github_reads::Reader>,
    supervisor: Arc<Supervisor>,
    store: SessionStore,
    history_loading: Arc<crate::history_loading::HistoryLoading>,
    names: Arc<Names>,
    channels: Arc<Channels>,
    gate: Gate<SupervisedTransport, DynConfirmer>,
    plugin_approvals: Arc<crate::plugin_approvals::PluginApprovals>,
    leases: Arc<crate::session_lease::Leases>,
    events: broadcast::Sender<CoreEvent>,
    config: CoreConfig,
    loads: mpsc::Sender<vw_session::Applied>,
    procmon: vw_procmon::ProcMon,
    terminals: vw_terminal::TerminalManager,
    instructions: crate::instructions::InstructionsPaths,
    agent_tools: Option<Arc<crate::agent_tools::AgentTools>>,
    preview: Option<Arc<crate::previews::PreviewHub>>,
    pages: Option<Arc<crate::pages::PagesHub>>,
    files: crate::files::FilesState,
    plan_profile: crate::plan_profile::PlanProfile,
    confirmer: DynConfirmer,
    sweep: Option<vw_procmon::SweepReport>,
    plugin_roots: Vec<std::path::PathBuf>,
}

/// Acquires the bookkeeping service before recovering plugins or starting the server.
fn session_leases(
    instructions: &crate::instructions::InstructionsPaths,
    config: &CoreConfig,
) -> Result<Arc<crate::session_lease::Leases>, CoreStartError> {
    crate::session_lease::Leases::new(&instructions.own_home, config.sessions_dir.as_deref())
        .map(Arc::new)
        .map_err(|error| CoreStartError::SessionOwnership(error.to_string()))
}

/// Recovers interrupted updates and installs the same pre-launch approval check before launching Vibe.
fn prepare_plugins(
    instructions: &crate::instructions::InstructionsPaths,
    supervisor: &mut SupervisorConfig,
    config: &CoreConfig,
) -> Result<
    (
        Arc<crate::plugin_approvals::PluginApprovals>,
        Vec<std::path::PathBuf>,
    ),
    CoreStartError,
> {
    vw_market::Store::new(&instructions.own_home).recover_updates()?;
    let plugin_approvals = Arc::new(
        crate::plugin_approvals::PluginApprovals::configured(supervisor, config)
            .map_err(|e| CoreStartError::PluginApprovals(e.to_string()))?,
    );
    let approvals = plugin_approvals.clone();
    supervisor.before_launch = Some(Arc::new(move |program| {
        let _held = approvals.mutations.try_lock().map_err(|e| e.to_string())?;
        approvals.before_launch(program).map_err(|e| e.to_string())
    }));
    let plugin_roots = crate::market_discovery::builtin_roots(supervisor);
    Ok((plugin_approvals, plugin_roots))
}

/// Publishes the prepared services with fresh per-run state; field order preserves constructor dependencies.
fn build_shared(state: Startup) -> Shared {
    let config = &state.config;
    let events = &state.events;
    let instructions = &state.instructions;
    Shared {
        browser_settings: crate::browser_settings::Settings::new(config),
        model_requests: state.model_requests,
        browser_dialogs: Arc::default(),
        vibe_processes: state.vibe_processes,
        deletes: state.deletes,
        app_context: state.app_context,
        asks: state.asks,
        ask_grants: crate::ask::grants::Grants::default(),
        chrome_control: state.chrome_control,
        install_offers: crate::install_offers::Offers::default(),
        guard_asks: crate::ask::guards::Calls::default(),
        identity_asks: crate::git_identity_ask::IdentityAsks::new(config.git_global_config.clone()),
        guard_refusals: crate::guard_refusals::Refusals::default(),
        feedback: crate::feedback::Feedback::new(config),
        edit_previews: crate::edit_preview::Previews::default(),
        protected_changes: crate::protected_changes::Hub::of(config),
        github_reads: state.github_reads,
        keep_awake: crate::keep_awake::from_config(config),
        new_chat: crate::new_chat::Draft::new(config.sessions_dir.as_deref()),
        skills_lock: tokio::sync::Mutex::new(()),
        queue_returns: crate::queue_return::Returns::default(),
        vibe_rules_lock: tokio::sync::Mutex::new(()),
        network_notice: std::sync::atomic::AtomicBool::new(config.network_settings_rejected),
        secret_scan: crate::secret_scan::ScanState::new(config.secret_scan_data.as_deref()),
        agent_access_locked: std::sync::atomic::AtomicBool::new(false),
        window_drive: std::sync::OnceLock::new(),
        supervisor: state.supervisor,
        store: state.store,
        history_loading: state.history_loading,
        entry_projects: crate::entry_projects::EntryProjects::default(),
        names: state.names,
        channels: state.channels,
        access: Access::load(config.sessions_dir.clone()),
        model_choices: crate::model_choices::Choices::load(config.sessions_dir.as_deref()),
        plugin_refresh: crate::plugin_refresh::Refresh::default(),
        interrupted_send: crate::interrupted_send::State::default(),
        stops: crate::stops::Stops::default(),
        stuck: crate::stuck::StuckSessions::default(),
        resync_reads: AtomicU64::new(0),
        gate: state.gate,
        // Built before `events` and `config` move into the struct (fields evaluate in this order).
        dictations: crate::dictation::Dictations::new(events.clone(), config.speech.clone()),
        voice_prompts: crate::voice_prompts::Prompts::default(),
        readings: crate::read_aloud::Pending::default(),
        panel_reads: crate::panel_read::Pending::default(),
        changes: crate::changes::Hub::from_config(config),
        worktrees: crate::worktrees::WorktreesState::of(config, instructions),
        git_browser: crate::git_browser_watch::BrowserWatches::default(),
        git: crate::git_cmd::GitHub::new(config.changes_dir.as_deref()),
        projects: crate::projects_filing::ProjectsState::new(config.projects_dir.clone()),
        market: crate::market::MarketState::new(&config.market, state.plugin_roots),
        plugin_approvals: state.plugin_approvals,
        // Only metadata from this app run is retained for the Settings use log.
        secret_uses: crate::secret_uses::SecretUses::default(),
        work: crate::work::WorkState::open(&config.work),
        temporary: Arc::new(crate::temporary::TemporarySessions::load(
            config.sessions_dir.clone(),
        )),
        search: crate::search_index::MessageIndex::default(),
        holders: crate::session_holder::Holders::default(),
        child_listings: crate::session_listing::ChildListings::default(),
        leases: state.leases,
        events: state.events,
        config: state.config,
        loads: state.loads,
        procmon: state.procmon,
        sweep: Mutex::new(state.sweep),
        ends: crate::session_end::Ends::default(),
        procmon_tick: Mutex::new(None),
        ledger_server_pid: std::sync::atomic::AtomicU32::new(0),
        terminals: state.terminals,
        instructions: state.instructions,
        documents: vw_documents::DocumentStore::default(),
        saved_documents: crate::saved_documents::SavedDocuments::default(),
        connectors: crate::connectors::ConnectorsState::default(),
        agent_tools: state.agent_tools,
        preview: state.preview,
        pages: state.pages,
        files: state.files,
        plan_profile: state.plan_profile,
        dialog_texts: state.confirmer.texts(),
        confirmer: state.confirmer,
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
    }
}

impl Core {
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
        let vibe_processes =
            crate::vibe_processes::VibeProcesses::start(&mut supervisor, &procmon).await?;
        let instructions = crate::instructions::InstructionsPaths {
            own_home: supervisor.vibe_home.clone(),
            cli_home: config.instructions.cli_home.clone(),
        };
        let leases = session_leases(&instructions, &config)?;
        let (plugin_approvals, plugin_roots) =
            prepare_plugins(&instructions, &mut supervisor, &config)?;
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
        let model_requests = secrets.model_requests();
        let supervisor = start_supervisor(supervisor, secrets, &config).await?;
        let first = supervisor.current().map(|conn| conn.subscribe());
        let (feeds, feeds_rx) = mpsc::unbounded_channel();
        let agent_tools = crate::agent_tools::open(
            config.agent_tools.clone(),
            &procmon,
            config.chrome_control.available(),
        );
        let store = SessionStore::new();
        let history_loading = Arc::new(crate::history_loading::HistoryLoading::new(store.clone()));
        let channels = Arc::new(Channels::new(
            supervisor.clone(),
            config.connection_wait,
            feeds,
            agent_tools.clone(),
            leases.clone(),
            plugin_approvals.clone(),
            history_loading.clone(),
        ));
        let transport = SupervisedTransport {
            channels: channels.clone(),
        };
        let names = Arc::new(Names::default());
        let (events, _) = broadcast::channel(config.event_capacity.max(1));
        let asks = crate::ask::Service::new(config.ask_clock.clone(), events.clone());
        let confirmer = DynConfirmer::new(confirmer, names.clone(), store.clone(), asks.clone());
        let gate = Gate::new(transport, confirmer.clone(), config.gate.clone());
        let terminals = crate::terminals::manager(&config, &procmon, &events);
        let (loads, loads_rx) = mpsc::channel(256);
        let (preview, pages, files) = crate::core_viewers::open(&config);
        let plan_profile = crate::plan_profile::PlanProfile::new(instructions.own_home.clone());
        let shared = Arc::new(build_shared(Startup {
            model_requests,
            vibe_processes,
            deletes,
            app_context,
            asks,
            chrome_control,
            github_reads,
            supervisor,
            store,
            history_loading,
            names,
            channels,
            gate,
            plugin_approvals,
            leases,
            events,
            config,
            loads,
            procmon,
            terminals,
            instructions,
            agent_tools,
            preview,
            pages,
            files,
            plan_profile,
            confirmer,
            sweep,
            plugin_roots,
        }));
        let _ = slot.set(Arc::downgrade(&shared));
        start_tasks(&shared, first, loads_rx, feeds_rx).await;
        Ok(Self { shared })
    }
}

/// Installs the initial pump, then starts services and finishes recovery before startup returns.
async fn start_tasks(
    shared: &Arc<Shared>,
    first: Option<vw_rpc::Subscription>,
    loads_rx: mpsc::Receiver<vw_session::Applied>,
    feeds_rx: mpsc::UnboundedReceiver<crate::channels::Feed>,
) {
    // Before any notification is applied: an open vault's values withhold titles from the first listing on.
    crate::title_guard::refresh(shared).await;
    let watcher = tokio::spawn(crate::title_guard::watch(
        Arc::downgrade(shared),
        shared.events.subscribe(),
    ));
    *shared
        .title_events
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(watcher);
    let task = tokio::spawn(pump::run(Arc::downgrade(shared), first, loads_rx, feeds_rx));
    *shared.pump.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
    spawn_key_forwarder(shared);
    connect_vibe_install(shared);
    crate::processes::note_vibe_server(shared);
    crate::processes::record_server(shared).await;
    // Session-list clients cannot run until startup returns after both cleanup sweeps.
    spawn_owned_tasks(shared);
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
    crate::recovery_schedules::start(shared);
    crate::keep_awake::publish(shared);
    crate::secret_scan::start(shared);
    // Repair app-owned browser packages in the background; startup does not wait.
    let refreshing = shared.clone();
    tokio::spawn(async move { crate::chrome_control::startup(&refreshing).await });
    crate::chrome_control::start_owner(shared).await;
    info!("core started");
}
