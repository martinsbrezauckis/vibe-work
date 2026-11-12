//! `Host`: starts and owns the core and the Debug API for the running app.
//!
//! Depends on: vw-core-api (`Core`), vw-debug-api (`DebugApi`), vw-supervisor, `bridge`, `forward`, `settings`.
//! Called by: `app` ([`Host::execute`], [`Host::subscribe`], [`Host::set_debug_api`], bridge), the test host.
//! Invariants:
//! - [`Host::start`] always returns a `Host`. When Vibe cannot be started the host records why in
//!   [`Startup`] and every command fails with `serverUnavailable` carrying that reason, so the
//!   window can open and say what is wrong instead of not opening at all.
//! - Every window command runs with `Origin::Ui` ([`Host::execute_window`]). The webview holds no Debug API
//!   authority: a command runs with `Origin::DebugApi` (which acts as the user without native dialogs) only when it
//!   matches a grant that a pending Debug API `ui/*` request got in Rust from its own route and body
//!   ([`crate::ui_grants`]), once, while the Debug API is on. Nothing the window sends (a flag, a token, an id) can
//!   claim that origin. While a request is served the core attributes window commands to the Debug API and holds
//!   them to its vault rule ([`UiBridge::driving`]); that only restricts.
//! - The Debug API is on in development builds and off in user builds unless the persisted
//!   setting says otherwise; turning it off removes the socket and the descriptor.

#[path = "host_routes.rs"]
mod route_descriptions;
#[path = "startup_keys.rs"]
mod startup_keys;
use route_descriptions::{BROWSER_ROUTES, CHANGES_ROUTES, PAGES_ROUTES, SHELL_ROUTES};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, warn};
use vw_core_api::{
    Command, CommandResult, Core, CoreConfig, CoreError, CoreEvent, ErrorCode, Origin,
};
use vw_debug_api::{DebugApi, UiRoutes};
use vw_platform::{Notifier, reveal_folder};
use vw_supervisor::{SecretProvider, SupervisorConfig};

use crate::bridge::UiBridge;
use crate::confirm::SharedConfirmer;
use crate::events::{HostEvent, HostEvents};
use crate::forward::Forwarder;
use crate::notify::{NotifyRefusal, NotifyRequest, RateLimit, prepare};
use crate::settings::HostSettings;

/// Default for how long a `ui/*` route waits for the UI to answer.
pub const DEFAULT_UI_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether Vibe is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// The core started and the server answered its handshake.
    Running,
    /// This build or machine has no way to start Vibe yet (no setup block, no dev server).
    Unavailable(String),
    /// A start was attempted and failed.
    Failed(String),
}

impl Startup {
    /// The reason text for the two non-running states.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Running => None,
            Self::Unavailable(r) | Self::Failed(r) => Some(r),
        }
    }
}

/// Everything [`Host::start`] needs.
pub struct HostConfig {
    /// The supervisor config, or the reason none could be built (shown in the UI).
    pub launch: Result<SupervisorConfig, String>,
    /// Where the API key comes from.
    pub secrets: Arc<dyn SecretProvider>,
    /// Core tuning (`Default` for production values).
    pub core: CoreConfig,
    /// `settings.json` (the Debug API switch).
    pub settings_path: PathBuf,
    /// Folder for the Debug API's per-run descriptor.
    pub descriptor_dir: PathBuf,
    /// Development build: the Debug API is on unless the setting says off.
    pub dev_build: bool,
    /// How long a `ui/*` route waits for the UI to answer ([`DEFAULT_UI_REPLY_TIMEOUT`]).
    pub ui_reply_timeout: Duration,
    /// Folders to register as user-picked at start. Tests only; in the app only the native folder
    /// dialog may do that ([`Host::register_picked_folder`]).
    pub picked_folders: Vec<PathBuf>,
    /// Shows the native confirmation dialogs (the app's dialog; [`SharedConfirmer::decline`] when
    /// none can be shown).
    pub confirmer: SharedConfirmer,
    /// Shows OS notifications and reports clicks (`vw_platform::DesktopNotifier` in the app).
    pub notifier: Arc<dyn Notifier>,
    /// The key service. When set, it starts before the core: the server gets
    /// its placeholder (`secrets` is replaced) and the `VIBE_*` overrides that route Mistral calls
    /// through the model proxy, and `keys` commands reach it. `None`: the key comes from `secrets`
    /// (tests and development runs with `MISTRAL_API_KEY` in the environment).
    pub keys: Option<vw_keys::KeyServiceConfig>,
    /// The app window for the Debug API's `/windows`, `/screenshot` and `/window`; `None` in runs
    /// without a window (test hosts, the browser dev UI).
    pub app_window: Option<vw_debug_api::SharedAppWindow>,
}

/// The running host. Cheap to share behind an `Arc`.
pub struct Host {
    pub(crate) core: Option<Core>,
    startup: Startup,
    pub(crate) ui: UiBridge,
    pub(crate) routes: UiRoutes,
    pub(crate) debug: AsyncMutex<Option<DebugApi>>,
    forwarder: Forwarder,
    runtime: Handle,
    notifier: Arc<dyn Notifier>,
    limiter: RateLimit,
    events: HostEvents,
    pub(crate) settings_path: PathBuf,
    pub(crate) descriptor_dir: PathBuf,
    pub(crate) dev_build: bool,
    pub(crate) keys: Option<Arc<vw_keys::KeyService>>,
    pub(crate) app_window: Option<vw_debug_api::SharedAppWindow>,
}

fn remove_legacy_debug_token(dir: &Path) {
    let test_export = vw_debug_api::DebugApiConfig::new(dir.to_path_buf()).test_token_file;
    if !test_export && let Err(error) = vw_debug_api::remove_legacy_token(dir) {
        warn!(%error, "could not remove legacy Debug API credentials");
    }
}

impl Host {
    /// Starts the core (when Vibe can be started) and the Debug API (when enabled).
    ///
    /// Must run inside a tokio runtime (Tauri's async runtime is one).
    pub async fn start(config: HostConfig) -> Self {
        // The startup state machine is large (about 16 KiB on the Windows target, every service's setup state):
        // boxed here once so every caller awaits a small future (clippy::large_futures with -D warnings).
        Box::pin(Self::start_inner(config)).await
    }

    async fn start_inner(config: HostConfig) -> Self {
        remove_legacy_debug_token(&config.descriptor_dir);
        let ui = UiBridge::new(config.ui_reply_timeout);
        let routes = UiRoutes::default();
        for (name, description) in SHELL_ROUTES
            .iter()
            .chain(crate::terminal::TERMINAL_ROUTES)
            .chain(CHANGES_ROUTES)
            .chain(PAGES_ROUTES)
            .chain(crate::files_routes::FILES_ROUTES)
            .chain(BROWSER_ROUTES)
        {
            if let Err(error) = ui.register_route(&routes, name, description) {
                warn!(%error, route = name, "ui route not registered");
            }
        }
        if let Err(error) = ui.register_client_routes(&routes) {
            warn!(%error, "ui client routes not registered");
        }
        let mut launch = config.launch;
        let mut secrets = config.secrets;
        let mut core_config = config.core;
        if !config.settings_path.as_os_str().is_empty() {
            core_config.secret_scan_data = config.settings_path.parent().map(Path::to_path_buf);
        }
        if let Some(cap) = HostSettings::load(&config.settings_path).terminal_cap() {
            core_config.terminals.max_per_session = cap;
        }
        let keys =
            startup_keys::start_keys(config.keys, &mut launch, &mut secrets, &mut core_config)
                .await;
        let (core, startup) = match launch {
            Err(reason) => {
                info!(%reason, "Vibe is not started in this run");
                (None, Startup::Unavailable(reason))
            }
            Ok(supervisor) => {
                match Core::start(supervisor, secrets, config.confirmer, core_config).await {
                    Ok(core) => (Some(core), Startup::Running),
                    Err(error) => {
                        warn!(%error, "the core did not start");
                        (None, Startup::Failed(error.to_string()))
                    }
                }
            }
        };
        if let Some(core) = &core {
            // Grants pin catalog facts; the core attributes window commands to the Debug API while it drives.
            ui.set_facts(Arc::new(crate::ui_grants::CoreFacts(core.clone())));
            let bridge = ui.clone();
            core.set_window_drive(move || bridge.driving());
            if let Some(window) = config.app_window.clone() {
                feedback_capture(core, window, routes.clone());
            }
            for folder in &config.picked_folders {
                if let Err(error) = core.register_picked_folder(folder) {
                    warn!(%error, "picked folder not registered");
                }
            }
        }
        let host = Self {
            core,
            startup,
            ui,
            routes,
            debug: AsyncMutex::new(None),
            forwarder: Forwarder::default(),
            runtime: Handle::current(),
            notifier: config.notifier,
            limiter: RateLimit::default(),
            events: HostEvents::new(),
            settings_path: config.settings_path,
            descriptor_dir: config.descriptor_dir,
            dev_build: config.dev_build,
            keys,
            app_window: config.app_window,
        };
        let setting = HostSettings::load(&host.settings_path).debug_api;
        host.start_debug_api(setting).await;
        host
    }

    /// Whether Vibe is running and, if not, why.
    #[must_use]
    pub fn startup(&self) -> &Startup {
        &self.startup
    }

    /// The core handle, when it is running (tests register folders and read status through it).
    #[must_use]
    pub fn core(&self) -> Option<&Core> {
        self.core.as_ref()
    }

    /// The key service, when this run has one (the private test host seeds a scratch vault through it
    /// during live runs; the app never needs it here).
    #[must_use]
    pub fn key_service(&self) -> Option<&Arc<vw_keys::KeyService>> {
        self.keys.as_ref()
    }

    /// The UI-control broker (the two Tauri commands call it).
    #[must_use]
    pub fn ui(&self) -> &UiBridge {
        &self.ui
    }

    /// The Debug API's descriptor path while it runs.
    pub async fn debug_descriptor(&self) -> Option<PathBuf> {
        self.debug
            .lock()
            .await
            .as_ref()
            .map(|api| api.descriptor_path().to_path_buf())
    }

    fn unavailable(&self) -> CoreError {
        let reason = self.startup.reason().unwrap_or("Vibe is not running");
        CoreError::new(ErrorCode::ServerUnavailable, reason)
    }

    /// The core, or `serverUnavailable` with the startup reason.
    pub(crate) fn core_or_unavailable(&self) -> Result<&Core, CoreError> {
        self.core.as_ref().ok_or_else(|| self.unavailable())
    }

    /// Executes one command for the app's own window (`origin = ui`).
    ///
    /// # Errors
    /// The core's [`CoreError`], or `serverUnavailable` with the startup reason when Vibe is not
    /// running.
    pub async fn execute(&self, command: Command) -> Result<CommandResult, CoreError> {
        self.execute_as(Origin::Ui, command).await
    }

    /// Executes one command the main window sent (the Tauri `core_execute`): `Origin::Ui`, unless it matches an unused
    /// grant of a pending Debug API `ui/*` request while the Debug API is on; then that one command runs with
    /// `Origin::DebugApi` and the grant is used up (module docs). Logged with the grant's name and the request id.
    ///
    /// # Errors
    /// As [`Host::execute`].
    pub async fn execute_window(&self, command: Command) -> Result<CommandResult, CoreError> {
        let granted = if self.debug.lock().await.is_some() {
            serde_json::to_value(&command)
                .ok()
                .and_then(|value| self.ui.take_grant(&value))
        } else {
            None
        };
        match granted {
            Some((request, grant)) => {
                info!(
                    command = command.name(),
                    grant,
                    request,
                    "a window command runs as the Debug API under its request's grant"
                );
                self.execute_as(Origin::DebugApi, command).await
            }
            None => self.execute_as(Origin::Ui, command).await,
        }
    }

    async fn execute_as(
        &self,
        origin: Origin,
        command: Command,
    ) -> Result<CommandResult, CoreError> {
        match &self.core {
            Some(core) => {
                let vault_command =
                    matches!(command, Command::Keys { .. } | Command::Secrets { .. });
                let result = core.execute(origin, command).await;
                if result.is_ok() && vault_command {
                    self.start_debug_api(HostSettings::load(&self.settings_path).debug_api)
                        .await;
                }
                result
            }
            None => Err(self.unavailable()),
        }
    }

    /// Starts forwarding core events to `sink` (first the snapshots, then changes), replacing a
    /// previous subscription. `sink` returns `false` when its receiver is gone.
    ///
    /// # Errors
    /// `serverUnavailable` when there is no core to subscribe to.
    pub fn subscribe<S>(&self, sink: S) -> Result<(), CoreError>
    where
        S: FnMut(CoreEvent) -> bool + Send + 'static,
    {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        self.forwarder
            .replace(&self.runtime, core.subscribe(None), sink);
        Ok(())
    }

    /// Records a folder the user picked in the native folder dialog (the only way a folder can
    /// become trustable) and returns the path as the dialog gave it, for the UI to use as a session
    /// folder. The webview never supplies the path: the app's Tauri command opens the dialog.
    ///
    /// # Errors
    /// `invalidParams` when the path is not an existing directory; `serverUnavailable` without a core.
    pub fn register_picked_folder(&self, path: &Path) -> Result<String, CoreError> {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        core.register_picked_folder(path).map_err(|error| {
            CoreError::new(
                ErrorCode::InvalidParams,
                format!("cannot use that folder: {error}"),
            )
        })?;
        Ok(path.display().to_string())
    }

    /// Shows a folder in the file manager when its canonical path is inside a listed or loaded
    /// session's folder or a saved project's main folder. A picked folder alone is not enough.
    ///
    /// # Errors
    /// `denied` for a path no session or project uses or a network path (Windows); `internal`
    /// when the file manager cannot start.
    pub fn reveal_folder(&self, path: &str) -> Result<(), CoreError> {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        let target = core.reveal_folder_target(Path::new(path))?;
        reveal_folder(&target).map_err(|error| {
            let error_code = match error {
                // A network or device path is never opened (it would authenticate to a server).
                vw_platform::RevealError::NotLocal(_) => ErrorCode::Denied,
                _ => ErrorCode::Internal,
            };
            CoreError::new(error_code, error.to_string())
        })
    }

    /// Opens a web link in the system browser after the user confirmed it in the link dialog.
    /// The main window never navigates; the URL is checked again here (http/https only, no
    /// user name, no whitespace or control characters) before any program starts.
    ///
    /// # Errors
    /// `invalidParams` for a link that may not be opened, `internal` when the browser cannot start.
    pub fn open_link(&self, url: &str) -> Result<(), CoreError> {
        vw_platform::open_web_url(url).map_err(|error| {
            let code = match error {
                vw_platform::OpenUrlError::Spawn(_) => ErrorCode::Internal,
                _ => ErrorCode::InvalidParams,
            };
            CoreError::new(code, error.to_string())
        })
    }

    /// Opens a `mailto:` link to one plain address in the system's mail program (the contact line in
    /// Settings › About). Only `mailto:` with a single address passes (`vw_platform::mail_url`); nothing is sent,
    /// the mail program only opens a new message.
    ///
    /// # Errors
    /// `invalidParams` for any other link, `internal` when the opener cannot start.
    pub fn open_mail_link(&self, url: &str) -> Result<(), CoreError> {
        vw_platform::open_mail_url(url).map_err(|error| {
            let code = match error {
                vw_platform::OpenUrlError::Spawn(_) => ErrorCode::Internal,
                _ => ErrorCode::InvalidParams,
            };
            CoreError::new(code, error.to_string())
        })
    }

    /// Shows an OS notification for the UI and publishes [`HostEvent::OpenSession`] if the user
    /// clicks it.
    ///
    /// # Errors
    /// `invalidParams` for a bad session id, `denied` when rate limited, `internal` when the OS
    /// cannot show it.
    pub fn notify(&self, request: &NotifyRequest) -> Result<(), CoreError> {
        let shown =
            prepare(request, &self.limiter, std::time::Instant::now()).map_err(|refusal| {
                match refusal {
                    NotifyRefusal::BadSession => {
                        CoreError::new(ErrorCode::InvalidParams, refusal.to_string())
                    }
                    NotifyRefusal::RateLimited => {
                        CoreError::new(ErrorCode::Denied, refusal.to_string())
                    }
                }
            })?;
        let publisher = self.events.publisher();
        let session_id = request.session_id.clone();
        let notification_id = request.notification_id;
        let on_click: Box<dyn FnOnce() + Send> = Box::new(move || {
            let _ = publisher.send(HostEvent::OpenSession {
                session_id,
                notification_id,
            });
        });
        let result = if let Some(id) = notification_id {
            self.notifier.show_tracked(id, shown, on_click)
        } else {
            self.notifier.show(shown, on_click)
        };
        result.map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))
    }

    /// Withdraws this app's OS notification after its ask is answered or stops waiting.
    pub fn dismiss_notification(&self, id: u32) {
        self.notifier.dismiss(id);
    }

    /// Starts forwarding [`HostEvent`]s to `sink`, replacing an earlier forwarder.
    pub fn subscribe_host<S>(&self, sink: S)
    where
        S: FnMut(HostEvent) -> bool + Send + 'static,
    {
        self.events.forward(&self.runtime, sink);
    }

    /// Delivers an OS update-notification activation to the UI.
    pub fn open_updates(&self) {
        let _ = self.events.publisher().send(HostEvent::OpenUpdates);
    }

    /// Delivers a native browser context-menu click as metadata only: Save to vault (the core still consumes the
    /// choice and requires native confirmation before storing anything) or, with `point`, Point Vibe at this.
    pub fn browser_context_menu(&self, session_id: String, point: bool) {
        let event = if point {
            HostEvent::BrowserPointContext { session_id }
        } else {
            HostEvent::BrowserSaveContext { session_id }
        };
        let _ = self.events.publisher().send(event);
    }

    /// Persists the Debug API switch and applies it now: starts the API when turned on, stops it
    /// (socket and descriptor gone) when turned off. The "Debug API on" flag event follows from
    /// the core.
    ///
    /// No dialog is needed to turn it on, in any build: the bearer token is the
    /// gate. A script in the webview can flip the switch but cannot read its vault value, so it
    /// gains nothing (and the token never crosses the bridge, see [`Host::debug_token_item`]).
    ///
    /// # Errors
    /// The I/O error when the setting cannot be saved (the switch is then not applied).
    pub async fn set_debug_api(&self, enabled: bool) -> std::io::Result<()> {
        let mut settings = HostSettings::load(&self.settings_path);
        settings.debug_api = Some(enabled);
        settings.save(&self.settings_path)?;
        let running = self.debug.lock().await.take();
        // Off or restarting: no window command runs as the Debug API under an earlier request's grant.
        self.ui.revoke_grants();
        if let Some(api) = running {
            if let Some(guard) = self.core.as_ref().and_then(Core::network_guard) {
                guard.remove_internal_port(api.port());
            }
            api.shutdown().await;
        }
        self.start_debug_api(Some(enabled)).await;
        Ok(())
    }

    /// The "keep Vibe Work sessions separate from the CLI" switch for Settings: the saved
    /// choice, whether the CLI home exists, and the CLI's version next to the bundled server's (the
    /// latter only while Vibe runs). Runs `vibe --version` (see [`crate::cli_sharing`]).
    pub async fn cli_sharing(
        &self,
        probe: &crate::cli_sharing::CliProbe,
    ) -> crate::cli_sharing::CliSharingView {
        let share = HostSettings::load(&self.settings_path).share_cli_sessions();
        let cli_version =
            crate::cli_sharing::detect_cli_version(probe, crate::cli_sharing::VERSION_TIMEOUT)
                .await;
        let bundled = self.core.as_ref().and_then(|c| c.status().vibe_version);
        let found = probe.cli_home.as_deref().is_some_and(Path::is_dir);
        crate::cli_sharing::CliSharingView::new(share, found, cli_version, bundled)
    }

    /// Saves the sharing switch. It applies the next time Vibe Work starts: the server's environment is
    /// fixed when it starts. Nothing else changes now.
    ///
    /// # Errors
    /// The I/O error when the setting cannot be saved.
    pub fn set_cli_sharing(&self, share: bool) -> std::io::Result<()> {
        let mut settings = HostSettings::load(&self.settings_path);
        settings.share_cli_sessions = Some(share);
        settings.save(&self.settings_path)
    }

    /// Stops the Debug API, the forwarder and the core (server shut down cleanly).
    pub async fn shutdown(&self) {
        self.forwarder.stop();
        self.events.stop();
        if let Some(api) = self.debug.lock().await.take() {
            api.shutdown().await;
        }
        if let Some(core) = &self.core {
            core.shutdown().await;
        }
        if let Some(keys) = &self.keys {
            keys.shutdown().await;
        }
    }
}

/// Lets the feedback form attach a screenshot of the app window, through the Debug API's capture (secret fields blanked).
fn feedback_capture(core: &Core, window: vw_debug_api::SharedAppWindow, routes: UiRoutes) {
    core.set_feedback_capture(move || {
        let window = window.clone();
        let routes = routes.clone();
        async move {
            vw_debug_api::capture_main(&window, &routes)
                .await
                .map(|(png, _)| png)
                .map_err(|error| {
                    if error.secret_check_failed() {
                        "Secret fields could not be hidden. No screenshot attached.".to_owned()
                    } else {
                        "Screenshot unavailable. No screenshot attached.".to_owned()
                    }
                })
        }
    });
}
