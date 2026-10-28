//! The user-build launch: how an installed Vibe Work starts `vibe-app-server`, assembled
//! here (not in the Tauri crate) so a test can run exactly this launch over the real Vibe with a scripted model
//! and read what the
//! server process got.
//!
//! What a user build gets:
//! - **Server:** Vibe Work's installers ship no Vibe. The server is the pinned Vibe
//!   the user installed at setup into `<data>/vibe/<version>/` (`vw_vibe_install`), or an existing install the
//!   user chose there. Before either exists the launch is still built: the core starts with the supervisor in
//!   `notInstalled` (nothing runs, the setup screen offers the install) and launches the server when the install
//!   finishes. Never a dev venv, never a binary from `PATH` the user did not choose.
//! - **Own `VIBE_HOME`** under the app's data folder, its `.env` kept secret-free ([`ProductLaunch`]); its
//!   `config.toml` names Mistral Medium 3.5 as the session-title model unless it names one already
//!   ([`crate::title_model`]), and Vibe verifies HTTPS with the OS certificate store unless it sets that itself
//!   ([`crate::trust_store`]).
//! - **Key:** only through the key service (model proxy + per-start placeholder); the parent's
//!   `MISTRAL_API_KEY` never reaches the server (the supervisor filters the parent environment, and the
//!   key service's provider replaces any other). If the key store cannot be located the launch fails
//!   with the reason instead of starting without protection.
//! - **No `PYTHON_KEYRING_BACKEND`:** Vibe keeps connector OAuth tokens in the OS keyring.
//! - **Sessions shared with the Vibe CLI** by default (`settings.json` `shareCliSessions`):
//!   `VIBE_SESSION_LOGGING__SAVE_DIR=<CLI home>/logs/session`.
//! - **Flags:** `--multiplex` (supervisor default) and
//!   `--additional-builtin-plugin-root <resources>/plugin` (only from the app's own files).
//! - **Session tools:** the bundled `vw-agent-tools` next to the app executable and the kill guard in our
//!   `hooks.toml` ([`crate::agent_tools::prepare`]).
//! - **Containment:** the supervisor's process group, watchdog and ledger (unchanged here).
//!
//! Also [`private_data_dir`]: the app's data folder, user-only, never a temp fallback (both build kinds).
//!
//! Depends on: `launch` ([`ProductLaunch`]), `agent_tools`, `settings`; vw-platform, vw-keys, vw-vault, vw-keyproxy,
//! vw-mistral (key service config). Called by: `app` (user builds), the private tests.
//! Invariant: every path comes from the caller (the app resolves them from Tauri; tests pass a scratch
//! layout); nothing here reads the process environment except through the supervisor at spawn time.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vw_core_api::CoreConfig;
use vw_keys::KeyServiceConfig;
use vw_keys::existing::{ExistingKeys, VibeKeyring};
use vw_supervisor::SupervisorConfig;
use vw_vault::location::VaultLocation;
use vw_vault::remember::DeviceSecretStore;

use crate::launch::ProductLaunch;
use crate::settings::HostSettings;

/// Folder of the built-in plugin root inside the bundle's resource folder (holds `vw/plugin.json`).
pub const PLUGIN_DIR: &str = "plugin";

/// The built-in plugin root under `resource_dir`.
#[must_use]
pub fn plugin_root(resource_dir: &Path) -> PathBuf {
    resource_dir.join(PLUGIN_DIR)
}

/// The app's data folder, made user-only. `resolved` is what the platform
/// gives (Tauri's `app_data_dir`); `dev_override` is the development builds' `VW_APP_DATA_DIR` (used only when
/// absolute; the app passes `None` in user builds). There is **no fallback**: a shared temp path would let
/// another local user pre-create the folder (or a symlink) and read or redirect the vault, settings, process
/// ledger and Debug API token. The folder is created 0700 / user-only DACL ([`vw_platform::ensure_private_dir`]);
/// a symlink, a non-directory or a folder owned by another user is refused.
///
/// # Errors
/// The text the app shows instead of starting Vibe: the folder cannot be resolved, or cannot be made private.
pub fn private_data_dir(
    resolved: Result<PathBuf, String>,
    dev_override: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let dir = match dev_override.filter(|p| p.is_absolute()) {
        Some(dir) => dir,
        None => resolved
            .map_err(|e| format!("Vibe Work can't find a private folder for its data: {e}"))?,
    };
    if !dir.is_absolute() {
        return Err(format!(
            "Vibe Work can't find a private folder for its data: {} is not an absolute path",
            dir.display()
        ));
    }
    vw_platform::ensure_private_dir(&dir).map_err(|e| {
        format!(
            "Vibe Work can't use {} as its private data folder: {e}",
            dir.display()
        )
    })?;
    Ok(dir)
}

/// Where the user's Vibe CLI keeps its home: `$VIBE_HOME` when the user set one (`vibe_home_env`), else
/// `<home>/.vibe`.
#[must_use]
pub fn cli_home(vibe_home_env: Option<PathBuf>, home: Option<&Path>) -> Option<PathBuf> {
    vibe_home_env
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".vibe")))
}

/// Everything a user-build launch needs, resolved by the caller.
pub struct ProductInputs {
    /// The bundle's resource folder (`None` when the platform could not resolve it).
    pub resource_dir: Option<PathBuf>,
    /// The running app executable (the `vw-agent-tools` sidecar sits next to it).
    pub exe: Option<PathBuf>,
    /// The app's data folder (`vibe-home/`, `work/`, `keys.json`, ... live under it).
    pub data_dir: PathBuf,
    /// The Vibe CLI's home ([`cli_home`]): session sharing and the "use Vibe's key" offer.
    pub cli_home: Option<PathBuf>,
    /// The vault profile folder (the app: the canonical ShellX profile), or why it cannot be used.
    pub vault: Result<VaultLocation, String>,
    /// Remembered-device secrets (the app: the OS credential store, memory when there is none).
    pub device_store: Arc<dyn DeviceSecretStore>,
    /// Where Vibe's own key may sit in the OS keyring (read only until the user acts).
    pub vibe_keyring: Arc<dyn VibeKeyring>,
    /// The key test call's base URL (the app: `vw_mistral::DEFAULT_API_BASE`).
    pub test_api_base: String,
    /// The model proxy's upstreams (the app: the defaults, Mistral's hosts).
    pub proxy: vw_keyproxy::ProxyConfig,
    /// The Vibe install: the installed or chosen server, or none yet. The app builds it over
    /// `<local app data>/vibe` with the embedded pin ([`vibe_installer`]); local, not roaming, on Windows (a
    /// ~450 MB install must not travel with a roaming profile).
    pub vibe: vw_vibe_install::VibeInstaller,
}

/// The Vibe installer of a user build: `<local_data_dir>/vibe` (Tauri's `app_local_data_dir`: `%LOCALAPPDATA%` on
/// Windows, the same folder as the data dir on Linux and macOS), the embedded pin for this OS, this process's
/// environment (the steps keep only the allowlisted names), discovery of existing installs from `PATH`.
#[must_use]
pub fn vibe_installer(local_data_dir: &Path) -> vw_vibe_install::VibeInstaller {
    vw_vibe_install::VibeInstaller::new(vw_vibe_install::InstallerConfig {
        root: local_data_dir.join("vibe"),
        sources: vw_vibe_install::Sources::pinned(&vw_vibe_install::this_target()),
        parent_env: std::env::vars_os().collect(),
        extra_env: Vec::new(),
        discovery: vw_supervisor::Discovery::from_env(None),
        limits: vw_vibe_install::Limits::default(),
    })
}

/// The assembled launch: the supervisor config (or why there is none) and the key service config.
pub struct ProductPlan {
    /// What the host starts.
    pub launch: Result<SupervisorConfig, String>,
    /// The key service (always set when `launch` is `Ok`).
    pub keys: Option<KeyServiceConfig>,
    /// Lines for the log (what was found, what is off and why).
    pub notes: Vec<String>,
}

/// Builds the user-build launch from `inputs` and the persisted `settings`; turns the session tools on
/// in `core` when the sidecar is bundled.
#[must_use]
pub fn plan(inputs: ProductInputs, settings: &HostSettings, core: &mut CoreConfig) -> ProductPlan {
    let mut notes = Vec::new();
    let Some(resources) = inputs.resource_dir else {
        return failed("the app's bundled files could not be located (resource folder unknown)");
    };
    // Tauri gives the resource folder as a verbatim path on Windows (`\\?\C:\...`, seen in the Windows 11 run as Vibe's
    // plugin root); Vibe and every path a user or agent reads get the plain form (`plain_path`).
    let resources = vw_platform::plain_path(&resources);
    let vault = match inputs.vault {
        Ok(vault) => vault,
        Err(reason) => return failed(&format!("the key store cannot be opened: {reason}")),
    };
    let cli_session_dir = if settings.share_cli_sessions() {
        inputs
            .cli_home
            .as_ref()
            .map(|h| h.join("logs").join("session"))
    } else {
        None
    };
    notes.push(match &cli_session_dir {
        Some(dir) => format!("sessions shared with the Vibe CLI: {}", dir.display()),
        None => "sessions kept separate from the Vibe CLI".to_owned(),
    });
    let installed = inputs.vibe.server();
    notes.push(match &installed {
        Some(server) => format!("Vibe: {}", server.display()),
        None => "Vibe is not installed yet: the setup screen offers the install".to_owned(),
    });
    // Before the install the supervisor waits; the path is only the place the install will put the server.
    let Some(server_bin) = installed.clone().or_else(|| inputs.vibe.expected_server()) else {
        return failed("this build's Vibe pin is not usable (no install folder)");
    };
    let product = ProductLaunch {
        server_bin,
        server_may_be_missing: installed.is_none(),
        app_data: inputs.data_dir.clone(),
        cli_session_dir,
    };
    core.vibe_install = Some(inputs.vibe.clone());
    let mut supervisor = match product.supervisor_config() {
        Ok(config) => config,
        Err(error) => return failed(&error.to_string()),
    };
    let sidecar = inputs
        .exe
        .as_deref()
        .and_then(crate::agent_tools::binary_next_to);
    // Settings checks the expected resource folder and reports missing or changed shipped bytes.
    core.chrome_control.bundled(&resources);
    if let Some(chrome) = &core.chrome_control.package {
        notes.push(format!("Chrome Control package: {}", chrome.display()));
    }
    let plugin = plugin_root(&resources);
    match crate::agent_tools::prepare(
        &mut supervisor,
        core,
        &inputs.data_dir,
        sidecar,
        Some(&plugin),
    ) {
        Ok(line) => notes.push(line),
        Err(error) => notes.push(format!(
            "agent tools off: hooks.toml could not be updated: {error}"
        )),
    }
    let keys = KeyServiceConfig {
        vault: vw_vault::VaultConfig {
            location: vault,
            device_store: inputs.device_store,
            kdf: vw_vault::KdfParams::default(),
            device_name: "Vibe Work".to_owned(),
        },
        managed_vault: Some(inputs.data_dir.clone()),
        proxy: inputs.proxy,
        test_api_base: inputs.test_api_base,
        settings_path: inputs.data_dir.join("keys.json"),
        existing: ExistingKeys {
            dotenv: inputs
                .cli_home
                .unwrap_or_else(|| inputs.data_dir.join("no-cli-home"))
                .join(".env"),
            keyring: inputs.vibe_keyring,
        },
    };
    ProductPlan {
        launch: Ok(supervisor),
        keys: Some(keys),
        notes,
    }
}

fn failed(reason: &str) -> ProductPlan {
    ProductPlan {
        launch: Err(reason.to_owned()),
        keys: None,
        notes: vec![format!("Vibe not started: {reason}")],
    }
}
