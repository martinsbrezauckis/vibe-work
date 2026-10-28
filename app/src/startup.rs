//! Builds the [`HostConfig`] for this run: where the data folder is, how Vibe is started, and the
//! Debug API defaults.
//!
//! Depends on: tauri (paths), vw-host. Called by: `lib.rs` (setup). Invariants: development
//! builds use [`DevLaunch`] (test `VIBE_HOME`, never `~/.vibe`); user builds use the product launch
//! ([`vw_host::product::plan`]): Vibe installed at setup into the data folder (or the
//! user's chosen install; none yet = the setup screen offers the install), the built-in plugin from the bundle's
//! resource folder, the `vw-agent-tools` sidecar next to the executable, our own `VIBE_HOME`, the key
//! service on the user's Vault profile, Vibe CLI session sharing per `settings.json` (default on); a user
//! build never takes a key from its own environment;
//! `VW_APP_DATA_DIR` moves the data folder in development builds only (tests need a known place for
//! the Debug API descriptor). `VW_DEV_KEYS=1` runs a development build with the key service (vault
//! profile, model proxy, placeholder key) in the dev run folder; development builds never use the
//! real Vault profile (`forbid_canonical_profile`). Other dev runs use a disposable placeholder.
//! The session tools are on when the
//! `vw-agent-tools` binary sits next to the app executable (`cargo build -p vw-agent-tools` in development):
//! core config, the kill guard in our `VIBE_HOME/hooks.toml`, and in development builds the built-in plugin
//! from `app/plugin` (`vw_host::agent_tools::prepare`).
//!
//! Test switches of development builds (ignored in user builds):
//! - `VW_DEV_SERVER_ARGS`: extra arguments for the dev server, split on whitespace (no quoting), so
//!   the real app can run pinned Vibe with its provider directed to a local scripted model.
//! - `VW_DEV_SERVER_LAUNCHER`: a launcher argv placed before the dev server, split the same way (a test's recording
//!   tap: `vibe_wire_tap --channels <file> --`). It is the supervisor's launcher, not the program, so the core
//!   recognizes the server below it as ours (`Supervisor::launcher_programs`); with the tap as the program, a
//!   subagent's lease owner (the server, a child of the tap) looked like another window and its card was refused.
//! - `VW_DEV_KEYS_STORE=os`: the unlocked vault's kept key goes to the real OS credential store (Windows
//!   Credential Manager, ...) instead of memory. The entry's account is derived from the dev run's
//!   own profile folder, so it can never be the real profile's entry.
//! - `VW_DEV_KEYS_API_BASE=<url>`: the key test call and the model proxy go to this base (a fake
//!   Mistral) instead of the real hosts.
//! - `VW_DEV_SLOW_NAMES=<suffix>`: host names ending in `<suffix>` take 2 s to resolve and answer a
//!   reserved address (refused), in the browser's name checks and its network guard, so a real-app test can show
//!   that slow lookups never freeze the window.
//! - `VW_DEV_CHECK_PUBLIC_NAMES=<suffix>`: the browser's name CHECKS (navigation hook, request barrier)
//!   answer a public address (`93.184.215.14`) for host names ending in `<suffix>`, while the engine and the network
//!   guard resolve them for real: a deterministic DNS rebinding (the check sees public, the connection private).

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use vw_core_api::CoreConfig;
use vw_host::launch::DevLaunch;
use vw_host::settings::HostSettings;
use vw_host::{DEFAULT_UI_REPLY_TIMEOUT, HostConfig, SharedConfirmer};
use vw_platform::DesktopNotifier;
use vw_supervisor::{NoSecret, SecretProvider};

/// The repository root of a development build: two levels above this crate's manifest folder.
const REPO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// True in development builds (`debug_assertions`), like the Debug API's own default.
pub const DEV_BUILD: bool = cfg!(debug_assertions);

/// The app's data folder, user-only (`vw_host::product::private_data_dir`): Tauri's app data folder, or
/// `VW_APP_DATA_DIR` in development builds only. No fallback: when it cannot be resolved or made private the
/// reason is returned and Vibe is not started.
///
/// # Errors
/// The text shown instead of starting Vibe.
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    data_dir_from(app.path().app_data_dir().map_err(|error| error.to_string()))
}

/// Resolves the standard desktop folder without initializing a GUI. Tauri directory overrides
/// still resolve in setup; live profiles and development data overrides work before setup too.
pub(crate) fn early_data_dir(config: &tauri::Config) -> Option<PathBuf> {
    let resolved = if config.app.app_directories_override.is_some() {
        Err("the app directory is resolved by Tauri".to_owned())
    } else {
        dirs::data_dir()
            .map(|path| path.join(&config.identifier))
            .ok_or_else(|| "the desktop data folder is unavailable".to_owned())
    };
    data_dir_from(resolved).ok()
}

fn data_dir_from(resolved: Result<PathBuf, String>) -> Result<PathBuf, String> {
    if let Some(root) = crate::live_profile::root()? {
        return vw_host::product::private_data_dir(Ok(root.join("data")), None);
    }
    vw_host::product::private_data_dir(resolved, dev_data_override())
}

/// A development run's own data folder (`VW_APP_DATA_DIR`, debug builds only). The run's local data and its main
/// window's web storage live next to it, so a test run never writes into an installed Vibe Work's per-user folders
/// (Windows `%LOCALAPPDATA%\app.vibework.desktop`: its `WebView2` profile, which an installed app may hold open).
pub(crate) fn dev_data_override() -> Option<PathBuf> {
    if DEV_BUILD {
        std::env::var_os("VW_APP_DATA_DIR").map(PathBuf::from)
    } else {
        None
    }
}

/// The folder the Vibe install goes into: Tauri's local app data folder, user-only like the data
/// folder. On Windows that is `%LOCALAPPDATA%\app.vibework.desktop` (a ~450 MB install must not roam with the
/// profile); on Linux and macOS it is the data folder itself. When it cannot be made private the data folder is
/// used (also private).
fn local_data_dir(app: &AppHandle, data: &std::path::Path) -> PathBuf {
    if let Ok(Some(root)) = crate::live_profile::root() {
        return vw_host::product::private_data_dir(Ok(root.join("local-data")), None)
            .unwrap_or_else(|_| data.to_path_buf());
    }
    if let Some(dev) = dev_data_override() {
        return vw_host::product::private_data_dir(Ok(dev.join("local-data")), None)
            .unwrap_or_else(|_| data.to_path_buf());
    }
    match vw_host::product::private_data_dir(
        app.path().app_local_data_dir().map_err(|e| e.to_string()),
        None,
    ) {
        Ok(local) => local,
        Err(reason) => {
            tracing::warn!(%reason, "local data folder unusable; Vibe is installed in the data folder");
            data.to_path_buf()
        }
    }
}

/// The host configuration when the data folder is unusable: no core, no key service, no Debug API, and no
/// path anything could be written to (the settings path is empty, so it reads as "defaults").
fn without_data(reason: String, confirmer: SharedConfirmer, app: &AppHandle) -> HostConfig {
    tracing::error!(%reason, "Vibe is not started");
    HostConfig {
        launch: Err(reason),
        secrets: Arc::new(NoSecret),
        core: CoreConfig::default(),
        settings_path: PathBuf::new(),
        descriptor_dir: PathBuf::new(),
        dev_build: DEV_BUILD,
        ui_reply_timeout: DEFAULT_UI_REPLY_TIMEOUT,
        picked_folders: Vec::new(),
        confirmer,
        notifier: Arc::new(DesktopNotifier::new(
            app.config().identifier.clone(),
            !DEV_BUILD,
        )),
        keys: None,
        app_window: None,
    }
}

/// The development launch ([`DevLaunch`]); user builds use [`product_launch`].
fn launch() -> Result<vw_supervisor::SupervisorConfig, String> {
    let mut config = DevLaunch::from_env(std::path::Path::new(REPO_ROOT))
        .and_then(|dev| dev.supervisor_config())
        .map_err(|error| error.to_string())?;
    config.extra_args.extend(dev_server_args(
        std::env::var("VW_DEV_SERVER_ARGS").ok().as_deref(),
    ));
    config.launcher = dev_server_args(std::env::var("VW_DEV_SERVER_LAUNCHER").ok().as_deref());
    Ok(config)
}

/// `VW_DEV_SERVER_ARGS` / `VW_DEV_SERVER_LAUNCHER` split on whitespace (empty or unset: none).
fn dev_server_args(value: Option<&str>) -> Vec<std::ffi::OsString> {
    value
        .unwrap_or_default()
        .split_whitespace()
        .map(std::ffi::OsString::from)
        .collect()
}

/// The remembered-device store of a dev run: the OS credential store with `VW_DEV_KEYS_STORE=os`
/// (falls back to memory, with a warning, when the platform has none), else memory.
fn dev_device_store() -> Arc<dyn vw_vault::remember::DeviceSecretStore> {
    if std::env::var_os("VW_DEV_KEYS_STORE").is_some_and(|v| v == "os") {
        match vw_vault::remember::OsDeviceStore::new() {
            Ok(store) => return Arc::new(store),
            Err(error) => {
                tracing::warn!(%error, "VW_DEV_KEYS_STORE=os: no OS credential store, using memory");
            }
        }
    }
    Arc::new(vw_vault::remember::MemoryDeviceStore::default())
}

/// The `vw-agent-tools` binary: next to the app executable (bundled with the app), else, in a
/// development build, the one cargo built in the same target folder (`cargo build -p vw-agent-tools`).
fn agent_tools_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    vw_host::agent_tools::binary_next_to(&exe)
}

/// The built-in plugin folder of a development build: the repository's `app/plugin`. User
/// builds load it from the bundle (`vw_host::product::plugin_root`).
fn plugin_root() -> Option<PathBuf> {
    DEV_BUILD.then(|| PathBuf::from(REPO_ROOT).join("app/plugin"))
}

/// Turns the session tools on for `launch` (core config, `hooks.toml`, plugin root); logs why when not.
fn with_agent_tools(
    launch: Result<vw_supervisor::SupervisorConfig, String>,
    core: &mut CoreConfig,
    data: &std::path::Path,
) -> Result<vw_supervisor::SupervisorConfig, String> {
    let mut supervisor = launch?;
    let plugin = plugin_root();
    match vw_host::agent_tools::prepare(
        &mut supervisor,
        core,
        data,
        agent_tools_binary(),
        plugin.as_deref(),
    ) {
        Ok(line) => tracing::info!("{line}"),
        Err(error) => tracing::warn!(%error, "agent tools off: hooks.toml could not be updated"),
    }
    Ok(supervisor)
}

/// The key service for a development run with `VW_DEV_KEYS=1`: shared vaults are read from the
/// dev run folder (`VW_DEV_VAULT_PROFILE_DIR` selects an isolated fixture folder), while new
/// vaults and their selection live in the app data folder. Remembered-device secrets stay in
/// memory (no OS keychain entries from dev runs unless `VW_DEV_KEYS_STORE=os`), Vibe's own key
/// is looked up only in the dev `VIBE_HOME`; the key test and
/// the proxy go to `VW_DEV_KEYS_API_BASE` when set.
fn dev_keys(data: &std::path::Path) -> Option<vw_keys::KeyServiceConfig> {
    if !DEV_BUILD || std::env::var_os("VW_DEV_KEYS").is_none_or(|v| v != "1") {
        return None;
    }
    let run = DevLaunch::from_env(std::path::Path::new(REPO_ROOT))
        .ok()?
        .run_dir;
    let api_base = std::env::var("VW_DEV_KEYS_API_BASE")
        .ok()
        .filter(|base| !base.trim().is_empty());
    let mut proxy = vw_keyproxy::ProxyConfig::default();
    if let Some(base) = &api_base {
        proxy.upstreams = vw_keyproxy::Upstreams {
            api: base.clone(),
            console: base.clone(),
            chat: base.clone(),
        };
    }
    Some(vw_keys::KeyServiceConfig {
        vault: vw_vault::VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(
                std::env::var_os("VW_DEV_VAULT_PROFILE_DIR")
                    .filter(|path| !path.is_empty())
                    .map_or_else(|| run.join("vault-profile"), PathBuf::from),
            ),
            device_store: dev_device_store(),
            kdf: vw_vault::KdfParams::default(),
            device_name: "vibe-work-dev".to_owned(),
        },
        managed_vault: Some(data.to_path_buf()),
        proxy,
        test_api_base: api_base.unwrap_or_else(|| vw_mistral::DEFAULT_API_BASE.to_owned()),
        settings_path: run.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: run.join("vibe-home/.env"),
            keyring: Arc::new(vw_keys::existing::MemoryVibeKeyring::default()),
        },
    })
}

/// The user's Vibe command-line home, for the instructions copy offer (read only). Development
/// builds look inside their own scratch home instead, never at the real `~/.vibe`.
pub(crate) fn cli_home(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(Some(root)) = crate::live_profile::root() {
        return Some(root.join("cli-vibe"));
    }
    if DEV_BUILD {
        let dev = DevLaunch::from_env(std::path::Path::new(REPO_ROOT)).ok()?;
        return Some(dev.run_dir.join("home").join(".vibe"));
    }
    vw_host::product::cli_home(
        std::env::var_os("VIBE_HOME").map(PathBuf::from),
        app.path().home_dir().ok().as_deref(),
    )
}

/// The OS credential store for the vault's kept key; memory (nothing kept across starts, so the
/// vault starts locked; logged) when the platform has none, e.g. Linux without a Secret Service.
fn os_device_store() -> Arc<dyn vw_vault::remember::DeviceSecretStore> {
    match vw_vault::remember::OsDeviceStore::new() {
        Ok(store) => Arc::new(store),
        Err(error) => {
            tracing::warn!(%error, "no OS credential store: the vault cannot stay unlocked across a restart");
            Arc::new(vw_vault::remember::MemoryDeviceStore::temporary())
        }
    }
}

/// Vibe's own key entries in the OS keyring (read for the import offer only); none without a store.
fn os_vibe_keyring() -> Arc<dyn vw_keys::existing::VibeKeyring> {
    match vw_keys::existing::OsVibeKeyring::new() {
        Ok(keyring) => Arc::new(keyring),
        Err(error) => {
            tracing::info!(%error, "no OS keyring: Vibe's own keyring entry is not looked up");
            Arc::new(vw_keys::existing::MemoryVibeKeyring::default())
        }
    }
}

/// The user build's Vibe installer, also handed to the update check (`updater::VibeSlot`).
fn vibe_installer(app: &AppHandle, data: &std::path::Path) -> vw_vibe_install::VibeInstaller {
    let installer = vw_host::product::vibe_installer(&local_data_dir(app, data));
    if let Some(slot) = app.try_state::<crate::updater::VibeSlot>() {
        let _ = slot.0.set(installer.clone());
    }
    installer
}

/// The user-build launch: what the product plan needs, resolved from Tauri and the OS.
fn product_launch(
    app: &AppHandle,
    data: &std::path::Path,
    core: &mut CoreConfig,
) -> (
    Result<vw_supervisor::SupervisorConfig, String>,
    Option<vw_keys::KeyServiceConfig>,
) {
    let settings = HostSettings::load(&data.join("settings.json"));
    let live_root = crate::live_profile::root().ok().flatten();
    let vault = live_root.as_ref().map_or_else(
        || vw_vault::location::VaultLocation::canonical().map_err(|e| e.to_string()),
        |root| {
            Ok(vw_vault::location::VaultLocation::explicit(
                root.join("shared-vault"),
            ))
        },
    );
    let vibe_keyring: Arc<dyn vw_keys::existing::VibeKeyring> = if live_root.is_some() {
        Arc::new(vw_keys::existing::MemoryVibeKeyring::default())
    } else {
        os_vibe_keyring()
    };
    let inputs = vw_host::product::ProductInputs {
        resource_dir: app.path().resource_dir().ok(),
        exe: std::env::current_exe().ok(),
        data_dir: data.to_path_buf(),
        cli_home: cli_home(app),
        vault,
        device_store: os_device_store(),
        vibe_keyring,
        test_api_base: vw_mistral::DEFAULT_API_BASE.to_owned(),
        proxy: vw_keyproxy::ProxyConfig::default(),
        vibe: vibe_installer(app, data),
    };
    let plan = vw_host::product::plan(inputs, &settings, core);
    for note in &plan.notes {
        tracing::info!("{note}");
    }
    (plan.launch, plan.keys)
}

/// The preview engine and settings: Tauri child webviews in the `main` window, one data folder per session. In a
/// development build the dev server's origin (`devUrl`) is refused as a preview URL (it would count as the app's
/// own origin for Tauri's ACL).
fn preview_setup(app: &AppHandle, data: &std::path::Path) -> vw_core_api::PreviewSetup {
    let mut config = vw_preview::PreviewConfig::new(data.join("preview"));
    // Leave room for cold creation, service-worker clearing (6 s), content rules (10 s) and the bounded canaries
    // (6 s). A late first canary must not lose to the outer engine-call timeout.
    config.engine_timeout = std::time::Duration::from_secs(25);
    #[cfg(feature = "e2e-script")]
    {
        config.test_browser_origin = std::env::var("VW_E2E_CONSENT_ORIGIN")
            .ok()
            .and_then(|origin| tauri::Url::parse(&origin).ok())
            .filter(|origin| {
                origin.scheme() == "http"
                    && matches!(origin.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                    && origin.port().is_some()
            });
        config.test_pool_origins = std::env::var("VW_E2E_BROWSER_POOL_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|text| tauri::Url::parse(text.trim()).ok())
            .filter(|url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some()
                    && url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .collect();
    }
    if DEV_BUILD {
        config
            .policy
            .dev_origin
            .clone_from(&app.config().build.dev_url);
    }
    // Previews require a running listener and proven canary on every platform; a bind error leaves them refused.
    let slow = DEV_BUILD
        .then(|| std::env::var("VW_DEV_SLOW_NAMES").ok())
        .flatten()
        .filter(|s| !s.is_empty());
    let resolver: Arc<dyn vw_netguard::Resolve> = match &slow {
        Some(suffix) => {
            config.name_resolver = Some(slow_names::name_resolver(suffix.clone()));
            Arc::new(slow_names::GuardResolver(suffix.clone()))
        }
        None => Arc::new(vw_netguard::SystemResolver),
    };
    if let Some(suffix) = DEV_BUILD
        .then(|| std::env::var("VW_DEV_CHECK_PUBLIC_NAMES").ok())
        .flatten()
        .filter(|s| !s.is_empty())
    {
        let inner = config
            .name_resolver
            .take()
            .unwrap_or_else(vw_preview::system_resolver);
        config.name_resolver = Some(slow_names::check_public(suffix, inner));
    }
    let guard = vw_netguard::NetGuard::bind(resolver)
        .inspect_err(|error| tracing::warn!(%error, "the browser's network guard did not start"))
        .ok();
    // Mark the previews as requiring the guard even when bind failed: the engine then gives a plain refusal.
    let mut engine = vw_webview::TauriEngine::new(app.clone(), "main").require_network_guard();
    let host = app.state::<vw_host::HostHandle>().inner().clone();
    engine = engine.with_context_sink(Arc::new(move |session_id, choice| {
        if let Some(host) = host.try_get() {
            host.browser_context_menu(session_id, choice == vw_webview::PageMenuChoice::PointVibe);
        }
    }));
    if let Some(guard) = &guard {
        engine = engine.with_network_guard(guard.clone());
    }
    vw_core_api::PreviewSetup {
        engine: Arc::new(engine),
        config,
        guard,
    }
}

/// Project Pages: the page viewer is a second engine instance (its own hooks: page-server navigation
/// only), one child webview in the `main` window; comments and per-project settings under `pages/`, the viewer's
/// website data under `pages-view/`. The visual check runs in a hidden webview of a further engine instance
/// (its own hooks: page-server navigation only; label `vw-preview-check-`, data under `pages-view/check`).
fn pages_setup(app: &AppHandle, data: &std::path::Path) -> vw_core_api::PagesSetup {
    vw_core_api::PagesSetup {
        engine: Arc::new(vw_webview::TauriEngine::new(app.clone(), "main")),
        config: vw_pages::PagesConfig::new(data.join("pages"), data.join("pages-view")),
        checker: Some(Arc::new(vw_webview::TauriEngine::new(app.clone(), "main"))),
    }
}

/// The File tab's viewer: a third engine instance with its own hooks and webview label prefix
/// (`vw-preview-files-`), showing one HTML page or PDF of a session's folder with the page server's isolation.
fn files_viewer_setup(app: &AppHandle, data: &std::path::Path) -> vw_core_api::PagesSetup {
    vw_core_api::PagesSetup {
        engine: Arc::new(vw_webview::TauriEngine::new(app.clone(), "main")),
        config: vw_pages::PagesConfig::for_files(
            data.join("files-viewer"),
            data.join("files-view"),
        ),
        checker: None,
    }
}

/// The host configuration for this run.
pub fn host_config(app: &AppHandle, confirmer: SharedConfirmer) -> HostConfig {
    let data = match data_dir(app) {
        Ok(data) => data,
        Err(reason) => return without_data(reason, confirmer, app),
    };
    if DEV_BUILD {
        vw_vault::location::forbid_canonical_profile();
    }
    let mut core = CoreConfig::default();
    let reading_app = app.clone();
    core.read_aloud_available = Some(Arc::new(move || {
        crate::main_window(&reading_app).is_some_and(|window| window.is_visible().unwrap_or(false))
    }));
    if let Some(index) = crate::startup_market::index() {
        core.market = vw_core_api::MarketConfig {
            index: Some(index),
            allow_loopback_http: true,
            ..Default::default()
        };
    }
    // About, `/health` and `ui/state` report the version the updater compares (the Tauri
    // config's, which the release scripts set), not the Cargo workspace's `0.0.0`.
    core.app_version = Some(app.package_info().version.to_string());
    // Process ledger: what sessions run survives a crash of the app and is swept next start.
    core.procmon.ledger_dir = Some(data.join("processes"));
    // Instructions: where "copy my CLI instructions" looks.
    core.instructions.cli_home = cli_home(app);
    // Changes tab: per-session review baselines, user-only.
    core.changes_dir = Some(data.join("changes"));
    // The user's project groups (`projects.json`, user-only).
    core.projects_dir = Some(data.join("projects"));
    // Which library each project may use and the image switch (`work.json`, user-only).
    core.work.dir = Some(data.join("work"));
    // Live image harnesses may pin the tool's conversation model as well as the chat model.
    // This development-only input never changes bundled application defaults.
    if DEV_BUILD && let Ok(model) = std::env::var("VW_DEV_IMAGE_MODEL") {
        tracing::info!(%model, "development image model override");
        core.work.image_model = model;
    }
    // Development runs: a Chrome Control package for Set up (debug builds only; user builds ship it in the bundle).
    // The store page is not opened from a dev run: its address goes to `<data>/chrome-store.log`.
    if let Some(package) = dev_chrome_package(DEV_BUILD, std::env::var_os("VW_DEV_CHROME_PACKAGE"))
    {
        tracing::info!(package = %package.display(), "development Chrome Control package");
        core.chrome_control.package = Some(package);
        let log = data.join("chrome-store.log");
        core.chrome_control.launcher = Some(Arc::new(move |url: &str| {
            use std::io::Write as _;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
                .and_then(|mut file| writeln!(file, "{url}"))
                .map_err(|e| e.to_string())
        }));
    }
    // The temporary sessions' record (deleted at quit, or at the next start after a crash).
    core.sessions_dir = Some(data.join("sessions"));
    // The key service verifies this agent-writable record through its vault before building any HTTPS client.
    core.network_file = Some(vw_mistral::network::settings_file(&data));
    // Preview pane: child webviews of the main window, website data per session under `preview/`.
    core.preview = Some(preview_setup(app, &data));
    // Project Pages: the page viewer, comments and per-project settings.
    core.pages = Some(pages_setup(app, &data));
    // The File tab: HTML pages and PDFs of a session's folder in an isolated viewer.
    core.files_viewer = Some(files_viewer_setup(app, &data));
    let (launch, keys, secrets): (_, _, Arc<dyn SecretProvider>) = if DEV_BUILD {
        (
            with_agent_tools(launch(), &mut core, &data),
            dev_keys(&data),
            vw_host::launch::placeholder_provider(),
        )
    } else {
        let (launch, keys) = product_launch(app, &data, &mut core);
        (launch, keys, Arc::new(NoSecret))
    };
    HostConfig {
        launch,
        secrets,
        core,
        settings_path: data.join("settings.json"),
        descriptor_dir: data.join("debug-api"),
        dev_build: DEV_BUILD,
        ui_reply_timeout: DEFAULT_UI_REPLY_TIMEOUT,
        picked_folders: Vec::new(),
        confirmer,
        notifier: Arc::new(DesktopNotifier::new(
            app.config().identifier.clone(),
            !DEV_BUILD,
        )),
        keys,
        app_window: Some(Arc::new(crate::window_api::TauriWindow::new(app.clone()))),
    }
}

/// Development runs only (`VW_DEV_CHROME_PACKAGE=<folder>`): the Chrome Control package Set up copies from.
/// User builds take it from the bundle's resources (`vw_host::product`); a dev run has no bundle, so without this
/// Set up says the build does not include the helper. Release builds and an empty value give none.
fn dev_chrome_package(dev_build: bool, value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .filter(|v| dev_build && !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{dev_chrome_package, dev_server_args};

    #[test]
    fn a_dev_chrome_package_only_in_development_builds_and_only_when_named() {
        assert_eq!(
            dev_chrome_package(true, Some("/pkg/chrome-control".into())),
            Some(std::path::PathBuf::from("/pkg/chrome-control"))
        );
        assert_eq!(
            dev_chrome_package(false, Some("/pkg".into())),
            None,
            "never in a release build"
        );
        assert_eq!(dev_chrome_package(true, Some("".into())), None);
        assert_eq!(dev_chrome_package(true, None), None);
    }

    #[test]
    fn dev_server_args_split_on_whitespace() {
        let none: Vec<std::ffi::OsString> = Vec::new();
        assert_eq!(dev_server_args(None), none);
        assert_eq!(dev_server_args(Some("  ")), none);
        assert_eq!(
            dev_server_args(Some(" --script  --status-events\t--rich-turn ")),
            ["--script", "--status-events", "--rich-turn"]
        );
    }
}

/// `VW_DEV_SLOW_NAMES` (development builds only): resolvers that answer names ending in a suffix after 2 s with a
/// reserved address (`192.0.2.1`, refused) and every other name through the system resolver.
mod slow_names {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use std::sync::Arc;
    use std::time::Duration;
    const DELAY: Duration = Duration::from_secs(2);
    const ANSWER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

    fn slow(host: &str, suffix: &str) -> bool {
        host.trim_end_matches('.')
            .to_ascii_lowercase()
            .ends_with(suffix)
    }

    /// For the navigation hooks and the request barrier (a blocking resolver, run on helper threads).
    pub(super) fn name_resolver(suffix: String) -> vw_preview::NameResolver {
        let system = vw_preview::system_resolver();
        Arc::new(move |host| {
            if slow(host, &suffix) {
                std::thread::sleep(DELAY);
                Ok(vec![ANSWER])
            } else {
                system(host)
            }
        })
    }

    /// `VW_DEV_CHECK_PUBLIC_NAMES`: the name checks see a public address for names ending in `suffix`; every other
    /// name goes to `inner` (the guard keeps the system resolver: the check and the connection disagree).
    pub(super) fn check_public(
        suffix: String,
        inner: vw_preview::NameResolver,
    ) -> vw_preview::NameResolver {
        Arc::new(move |host| {
            if slow(host, &suffix) {
                Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14))])
            } else {
                inner(host)
            }
        })
    }

    /// For the network guard (async).
    pub(super) struct GuardResolver(pub(super) String);

    impl vw_netguard::Resolve for GuardResolver {
        fn resolve<'a>(&'a self, host: &'a str, port: u16) -> vw_netguard::ResolveFuture<'a> {
            Box::pin(async move {
                if slow(host, &self.0) {
                    tokio::time::sleep(DELAY).await;
                    return Ok(vec![SocketAddr::new(ANSWER, port)]);
                }
                vw_netguard::SystemResolver.resolve(host, port).await
            })
        }
    }
}
