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
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tracing::info;
use vw_gate::{Confirmer, Gate, GateConfig, GateError};
use vw_session::{Applied, SessionStore};
use vw_supervisor::{SecretProvider, Supervisor, SupervisorConfig, SupervisorError};
use vw_terminal::{PortLeaseSource, TerminalConfig, TerminalManager};

use crate::access::Access;
use crate::channels::Channels;
use crate::confirmer::DynConfirmer;
pub use crate::error::CoreStartError;
use crate::event::CoreEvent;
use crate::names::Names;
use crate::pump;
use crate::subscription::CoreSubscription;
use crate::transport::SupervisedTransport;
use crate::view::{
    ChannelView, ChannelsReport, CoreStatus, DebugApiFlag, ServerReport, ServerStatus, duration_ms,
};

/// Tuning of a core. `Default` gives production values.
#[derive(Clone)]
#[non_exhaustive]
pub struct CoreConfig {
    /// The gate's tuning (token lifetime, limits, clock).
    pub gate: GateConfig,
    /// How long a command waits for a ready server before failing with `serverUnavailable`.
    /// Default 10 s.
    pub connection_wait: Duration,
    /// Undelivered store items kept per subscriber before it is resnapshotted. Default 1024.
    pub subscriber_capacity: usize,
    /// Core events buffered per subscriber before it is told it lagged. Default 256.
    pub event_capacity: usize,
    /// Stderr lines in a server report. Default 40.
    pub stderr_lines: usize,
    /// Commands the user types in the app's own window (`session/shellCommand` from
    /// [`Origin::Ui`](crate::Origin)) and the typed composer Full access switch run without the native dialog,
    /// because the main window is a
    /// trusted surface that never renders untrusted active content. Trusted-surface tests
    /// cover this requirement. Default `true`. Other methods keep the
    /// window's dialog regardless. The Debug API still asks natively for Full access.
    pub trusted_ui_shell: bool,
    /// How long a session's channel stays open after its last use when the session is quiet and no
    /// client views it (`recovery.rs`). Closing releases the session's lease in Vibe, so the
    /// Vibe CLI can take the session, and frees the channel's memory in the server; the next command
    /// for the session reopens it (`session/resume`, well under a second when no crash lock is held).
    /// Default 5 minutes: long enough that switching between a few sessions never reopens them,
    /// short enough that a session left alone is free for the CLI soon.
    pub channel_idle_close: Duration,
    /// Longest wait for reopening a session after a server restart; past it (or on an error) the
    /// session is stuck: its commands are refused until the user acts (no automatic
    /// retries, a user must never wait 150 s mid-work). Default 60 s (an observed resume took ~31 s after a
    /// kill during a running tool).
    pub reopen_timeout: Duration,
    /// Longest wait for one `retryReopen` attempt. Default 60 s.
    pub retry_reopen_timeout: Duration,
    /// The key service: answers `keys` commands, and its events become
    /// `keyNeeded` / `keyStatusChanged` core events. `None` (tests, development runs with the key
    /// in the environment): `keys` commands fail with `keyService` / `notConfigured`.
    pub keys: Option<Arc<vw_keys::KeyService>>,
    /// Native clipboard owner for user-only vault copy commands. The core shuts it down.
    pub clipboard: Option<Arc<vw_platform::clipboard::ClipboardService>>,
    /// The Vibe install at setup: answers `vibe` commands; its view changes become `vibeInstall`
    /// events. When it has no server yet (`VibeInstaller::server` is `None`), the core starts with the
    /// supervisor in `notInstalled` (nothing spawned) and starts the first server when the install finishes.
    /// `None` (tests, development builds): `vibe` commands fail with `notConfigured`.
    pub vibe_install: Option<vw_vibe_install::VibeInstaller>,
    /// Process and port monitor settings: lease range, stop timing, tick, ledger folder.
    /// `Default` has no ledger; the app sets `ledger_dir` to a folder in its data directory.
    pub procmon: vw_procmon::ProcMonConfig,
    /// Integrated terminals: per-session cap, scrollback, flow control, shell.
    pub terminals: TerminalConfig,
    /// Where a session's terminals get `PORT` from. Default `None`: the process monitor's
    /// per-session lease (the first leased port). Tests may set a fixed source.
    pub terminal_ports: Option<Arc<dyn PortLeaseSource>>,
    /// Instructions: where the user's command-line home is, for the copy offer.
    pub instructions: crate::instructions::InstructionsConfig,
    /// Documents: how long one OCR request may take.
    pub documents: crate::documents::DocumentsConfig,
    /// Voice: dictation length, idle drop, how long `stop` waits for the live text.
    pub speech: crate::dictation::SpeechConfig,
    /// Session-scoped agent tools: the per-session MCP server, `process_start` and the kill
    /// guard's endpoint. `None` (default): no MCP server is registered.
    pub agent_tools: Option<crate::agent_tools::AgentToolsConfig>,
    /// Where the Changes tab keeps review baselines (made user-only). `None` (the default): the
    /// Changes tab is off. The app sets a folder in its data directory.
    pub changes_dir: Option<std::path::PathBuf>,
    /// Longest a session's review baseline may take before its first piece of work is sent to Vibe without
    /// one. A baseline still running at this point is discarded when it ends (taken late, it could
    /// contain the agent's first edits) and the review is reported unavailable until the user starts it
    /// again. Default 15 s.
    pub changes_baseline_limit: Duration,
    /// Test switch: calls around the baseline taken before a session's first work, so a test can
    /// hold it past `changes_baseline_limit` and know when it ended. Default `None`.
    pub changes_test_hooks: Option<Arc<dyn crate::changes_slots::ChangesTestHooks>>,
    /// The preview pane: the native engine and its settings. `None` (the default): no previews.
    pub preview: Option<crate::previews::PreviewSetup>,
    /// Project Pages: the viewer engine and the app-data folders. `None` (the default): no Pages.
    pub pages: Option<crate::pages::PagesSetup>,
    /// The user's project groups: the folder of `projects.json`. `None` (the default): the groups live
    /// in memory for this run only. The app sets a folder in its data directory.
    pub projects_dir: Option<std::path::PathBuf>,
    /// Session bookkeeping of Vibe Work's own: the temporary sessions' record. `None` (the default):
    /// memory only, so temporary sessions of a crashed run are not deleted at the next start.
    pub sessions_dir: Option<std::path::PathBuf>,
    /// The File tab's viewer for HTML pages and PDFs: a third engine instance whose config uses
    /// `vw_pages::FILES_LABEL_PREFIX`. `None` (the default): HTML and PDF files show their source / a notice only.
    pub files_viewer: Option<crate::pages::PagesSetup>,
    /// The version this build reports (Settings > About, `GET /health`, `ui/state`). The app
    /// passes Tauri's `package_info().version`, the version the updater compares, so all of them agree (the
    /// release scripts set it in the Tauri config overlay; the Cargo workspace version stays `0.0.0`). `None`
    /// (tests, the default): this crate's own version.
    pub app_version: Option<String>,
    /// gh to use for pull requests and the PR card. `None` (the default): the first `gh` on `PATH` or
    /// in the usual install folders. Tests point it at a fake.
    pub gh_program: Option<std::path::PathBuf>,
    /// GitHub's REST API base for the PR card without gh. Default `https://api.github.com`; tests
    /// point it at a local fake.
    pub github_api_base: String,
    /// The model that drafts commit messages. Default [`crate::git_cmd::DEFAULT_COMMIT_MODEL`].
    pub commit_model: String,
    /// The shortest time between two forge reads of one PR card. `None` (the default): 30 s with gh,
    /// 120 s with GitHub's public API. Tests set it to zero.
    pub pr_card_floor: Option<Duration>,
    /// The Tools marketplace: a test catalog and downloader; the default is the built-in catalog over
    /// HTTPS.
    pub market: crate::market::MarketConfig,
    /// Libraries and image generation: `work.json`'s folder, the models, the image download policy.
    pub work: crate::work::WorkConfig,
}

impl CoreConfig {
    /// How often the pump looks for idle channels: a quarter of `channel_idle_close`, between 10 ms
    /// (tests) and 30 s.
    pub(crate) fn idle_check(&self) -> Duration {
        (self.channel_idle_close / 4).clamp(Duration::from_millis(10), crate::pump::MAX_IDLE_CHECK)
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            gate: GateConfig::default(),
            connection_wait: Duration::from_secs(10),
            subscriber_capacity: 1024,
            event_capacity: 256,
            stderr_lines: 40,
            trusted_ui_shell: true,
            channel_idle_close: Duration::from_secs(300),
            reopen_timeout: Duration::from_secs(60),
            retry_reopen_timeout: Duration::from_secs(60),
            keys: None,
            clipboard: None,
            vibe_install: None,
            procmon: vw_procmon::ProcMonConfig::default(),
            terminals: TerminalConfig::default(),
            terminal_ports: None,
            instructions: crate::instructions::InstructionsConfig::default(),
            documents: crate::documents::DocumentsConfig::default(),
            speech: crate::dictation::SpeechConfig::default(),
            agent_tools: None,
            changes_dir: None,
            changes_baseline_limit: crate::changes::BASELINE_LIMIT,
            changes_test_hooks: None,
            preview: None,
            pages: None,
            projects_dir: None,
            sessions_dir: None,
            files_viewer: None,
            app_version: None,
            gh_program: None,
            github_api_base: "https://api.github.com".to_owned(),
            commit_model: crate::git_cmd::DEFAULT_COMMIT_MODEL.to_owned(),
            pr_card_floor: None,
            market: crate::market::MarketConfig::default(),
            work: crate::work::WorkConfig::default(),
        }
    }
}

/// The native-view features the app configured: previews, the Project tab's viewer and the File tab's
/// viewer.
fn viewers(
    config: &CoreConfig,
) -> (
    Option<Arc<crate::previews::PreviewHub>>,
    Option<Arc<crate::pages::PagesHub>>,
    crate::files::FilesState,
) {
    (
        crate::previews::open(config.preview.clone()),
        crate::pages::open(config.pages.clone()),
        crate::files::FilesState::new(crate::pages::open(config.files_viewer.clone())),
    )
}

/// State shared by every handle, the pump and the server-request handler.
pub(crate) struct Shared {
    /// User-controlled agent access pause; separate from cryptographic vault lock.
    pub(crate) agent_access_locked: std::sync::atomic::AtomicBool,
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
    /// Sessions Vibe could not reopen after a restart (`stuck.rs`).
    pub(crate) stuck: crate::stuck::StuckSessions,
    /// `session/read` resyncs the pump started (status report; asserts none while sessions run).
    pub(crate) resync_reads: AtomicU64,
    pub(crate) gate: Gate<SupervisedTransport, DynConfirmer>,
    pub(crate) events: broadcast::Sender<CoreEvent>,
    pub(crate) config: CoreConfig,
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
    /// Running dictations (`dictation.rs`).
    pub(crate) dictations: Arc<crate::dictation::Dictations>,
    /// The running server sign-in and the catalog-loaded flag (`connectors.rs`).
    pub(crate) connectors: crate::connectors::ConnectorsState,
    /// The marketplace's catalog, downloader and running install (`market.rs`).
    pub(crate) market: crate::market::MarketState,
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
    pub(crate) temporary: crate::temporary::TemporarySessions,
    /// The message index of the search (`search_index.rs`).
    pub(crate) search: crate::search_index::MessageIndex,
    /// Sessions another process holds (`session_holder.rs`), and the task that checks the viewed ones.
    pub(crate) holders: crate::session_holder::Holders,
    /// The outer git loop (`git_cmd.rs`): data folders, the one-at-a-time lock, PR cards.
    pub(crate) git: crate::git_cmd::GitHub,
    /// The secret broker's "always" grants and use log (`secret_grants.rs`).
    pub(crate) secret_grants: crate::secret_grants::SecretGrants,
    /// Libraries' scopes, the image switch and image counters (`work.rs`).
    pub(crate) work: crate::work::WorkState,
    started: Instant,
    debug_api: watch::Sender<DebugApiFlag>,
    pump: Mutex<Option<JoinHandle<()>>>,
    /// Forwards key-service events as core events (owned, aborted with the pump).
    key_events: Mutex<Option<JoinHandle<()>>>,
    /// Re-emits the Vibe install's view changes.
    vibe_events: Mutex<Option<JoinHandle<()>>>,
}

impl Shared {
    /// Publishes a core event to every subscriber (none listening is fine).
    pub(crate) fn emit(&self, event: CoreEvent) {
        let _ = self.events.send(event);
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if let Some(tick) = self
            .procmon_tick
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            tick.abort();
        }
        for slot in [&self.pump, &self.key_events, &self.vibe_events] {
            if let Some(task) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
                task.abort();
            }
        }
    }
}

/// Re-emits the key service's events as core events until the core goes away.
async fn forward_key_events(
    shared: Weak<Shared>,
    mut events: tokio::sync::broadcast::Receiver<vw_keys::KeyEvent>,
) {
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                vw_keys::KeyEvent::StatusChanged
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let Some(shared) = shared.upgrade() else {
            return;
        };
        if event == vw_keys::KeyEvent::KeyChanged {
            crate::recovery::reopen_after_key_change(&shared).await;
        }
        shared.emit(match event {
            vw_keys::KeyEvent::KeyNeeded(need) => CoreEvent::KeyNeeded {
                reason: match need {
                    vw_keyproxy::KeyNeed::Unlock => "unlock",
                    vw_keyproxy::KeyNeed::Setup => "setup",
                }
                .to_owned(),
            },
            vw_keys::KeyEvent::RateLimits(limits) => CoreEvent::RateLimits {
                limits: limits.into(),
            },
            vw_keys::KeyEvent::StatusChanged | vw_keys::KeyEvent::KeyChanged => {
                CoreEvent::KeyStatusChanged {}
            }
        });
    }
}

/// The supervisor: started with its server, or waiting with no server (nothing spawned) when Vibe is
/// not installed yet; the install's `on_ready` hook launches it later ([`connect_vibe_install`]).
async fn start_supervisor(
    supervisor: SupervisorConfig,
    secrets: Arc<dyn SecretProvider>,
    config: &CoreConfig,
) -> Result<Arc<Supervisor>, SupervisorError> {
    crate::mcp_keys::prepare_config(&supervisor.vibe_home, config.keys.as_ref()); // Use this run's port.
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
        let forwarder = tokio::spawn(forward_key_events(Arc::downgrade(shared), keys.subscribe()));
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

/// The startup sweep of the process ledger: stops owned leftovers of an earlier run,
/// re-attaches agent processes. Runs off the async runtime; a failure is logged, never fatal.
async fn startup_sweep(procmon: &vw_procmon::ProcMon) -> Option<vw_procmon::SweepReport> {
    let monitor = procmon.clone();
    // Session existence is unknown before the first listing: nothing is called "deleted" on a guess.
    tokio::task::spawn_blocking(move || monitor.sweep(&|_| None))
        .await
        .ok()
        .and_then(|r| {
            r.map_err(|e| tracing::warn!(error = %e, "startup sweep failed"))
                .ok()
        })
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
    /// the ledger folder cannot be prepared.
    pub async fn start(
        mut supervisor: SupervisorConfig,
        secrets: Arc<dyn SecretProvider>,
        confirmer: impl Confirmer + 'static,
        config: CoreConfig,
    ) -> Result<Self, CoreStartError> {
        let config = crate::secrets::configure_clipboard(config)?;
        let procmon = vw_procmon::ProcMon::new(config.procmon.clone())?;
        let sweep = startup_sweep(&procmon).await;
        let instructions = crate::instructions::InstructionsPaths {
            own_home: supervisor.vibe_home.clone(),
            cli_home: config.instructions.cli_home.clone(),
        };
        let slot = own_callbacks(&mut supervisor);
        let supervisor = start_supervisor(supervisor, secrets, &config).await?;
        let first = supervisor.current().map(|conn| conn.subscribe());
        let (feeds, feeds_rx) = mpsc::unbounded_channel();
        let agent_tools = crate::agent_tools::open(config.agent_tools.clone(), &procmon);
        let channels = Arc::new(Channels::new(
            supervisor.clone(),
            config.connection_wait,
            feeds,
            agent_tools.clone(),
        ));
        let transport = SupervisedTransport {
            channels: channels.clone(),
        };
        let store = SessionStore::new();
        let names = Arc::new(Names::default());
        let confirmer = DynConfirmer::new(confirmer, names.clone(), store.clone());
        let gate = Gate::new(transport, confirmer.clone(), config.gate.clone());
        let (events, _) = broadcast::channel(config.event_capacity.max(1));
        let terminals = crate::terminals::manager(&config, &procmon, &events);
        let (loads, loads_rx) = mpsc::channel(256);
        let (preview, pages, files) = viewers(&config);
        let plan_profile = crate::plan_profile::PlanProfile::new(instructions.own_home.clone());
        let shared = Arc::new(Shared {
            agent_access_locked: std::sync::atomic::AtomicBool::new(false),
            supervisor,
            store,
            entry_projects: crate::entry_projects::EntryProjects::default(),
            names,
            channels,
            access: Access::load(config.sessions_dir.clone()),
            stuck: crate::stuck::StuckSessions::default(),
            resync_reads: AtomicU64::new(0),
            gate,
            // Built before `events` and `config` move into the struct (fields evaluate in this order).
            dictations: crate::dictation::Dictations::new(events.clone(), config.speech.clone()),
            changes: crate::changes::Hub::from_config(&config),
            worktrees: crate::worktrees::WorktreesState::of(&config, &instructions),
            git: crate::git_cmd::GitHub::new(config.changes_dir.as_deref()),
            projects: crate::projects_filing::ProjectsState::new(config.projects_dir.clone()),
            market: crate::market::MarketState::new(&config.market),
            // The grants sit in the agent tools' data folder (opened private above), memory only without.
            secret_grants: crate::secret_grants::SecretGrants::open(
                agent_tools.as_ref().map(|t| t.config.data_dir.as_path()),
            ),
            work: crate::work::WorkState::open(&config.work),
            temporary: crate::temporary::TemporarySessions::load(config.sessions_dir.clone()),
            search: crate::search_index::MessageIndex::default(),
            holders: crate::session_holder::Holders::default(),
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
        });
        let _ = slot.set(Arc::downgrade(&shared));
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
        spawn_owned_tasks(&shared);
        crate::temporary::spawn_sweep(&shared);
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

    /// Stops the pump, stops every session's `owned` processes (the monitor; `agent` processes keep
    /// running and stay in the ledger), returns the leases, then shuts the server down cleanly (stdin
    /// closed, grace, then kill).
    pub async fn shutdown(&self) {
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
        self.delete_temporary_all().await;
        let exit = self.shared.supervisor.stop().await;
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
