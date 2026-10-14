//! Development launch config: how a development build of the app starts `vibe-app-server`
//! with its own isolated Vibe home.
//!
//! Rules: a dev run uses its own `VIBE_HOME` and `HOME` under one run folder,
//! never the user's `~/.vibe`; the server binary and the config template come from the repo's
//! git-ignored `data/` (or the environment variables below, for machines that have no `data/`).
//! Live development uses the key service; other runs get a disposable placeholder.
//!
//! Environment (all optional, absolute paths):
//! - `VW_DEV_SERVER_BIN`: the `vibe-app-server` executable (default `<repo>/data/venv/bin/...`).
//! - `VW_DEV_HOME_TEMPLATE`: a `config.toml` copied into the dev `VIBE_HOME`
//!   (default `<repo>/data/vibe-home-template/config.toml`).
//! - `VW_DEV_RUN_DIR`: the run folder (default `<repo>/.scratch/dev-app`, git-ignored).
//!
//! Depends on: vw-supervisor (`SupervisorConfig`), vw-keys (`.env` key removal). Called by: `app` in
//! development builds (user builds use [`ProductLaunch`] through [`crate::product`]), the
//! private test host. Invariants: nothing outside the run folder is written; an existing dev
//! `config.toml` keeps user edits; relative extension entries are dropped and a session-title model is added when it
//! names none ([`crate::title_model`]; both launches). Config parsing never fails launch; the dev server runs unsandboxed, so
//! live development uses the same vault and proxy flow as a user build.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use vw_supervisor::SupervisorConfig;

/// A disposable placeholder for development without a key service. It prevents Vibe from falling back to
/// provider keys in the parent environment, a dotenv file or its OS credential store.
#[must_use]
pub fn placeholder_provider() -> std::sync::Arc<vw_keyproxy::ProxySecretProvider> {
    std::sync::Arc::new(vw_keyproxy::ProxySecretProvider::new(std::sync::Arc::new(
        vw_keyproxy::TokenSlot::default(),
    )))
}

/// Why the development launch config could not be built.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// The server executable does not exist at the resolved path.
    #[error(
        "vibe-app-server not found at {0}: install the pinned Vibe into data/venv (tools/public/codegen/vibe.pin) or set VW_DEV_SERVER_BIN"
    )]
    ServerMissing(PathBuf),
    /// The config template does not exist at the resolved path.
    #[error("dev VIBE_HOME template not found at {0}: set VW_DEV_HOME_TEMPLATE to a config.toml")]
    TemplateMissing(PathBuf),
    /// A variable was set to a relative path (only absolute paths are accepted).
    #[error("{0} must be an absolute path")]
    NotAbsolute(&'static str),
    /// Preparing a launch folder or config file failed.
    #[error("cannot prepare the launch path {path}: {source}")]
    Prepare {
        /// The folder or file involved.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
}

/// Resolved inputs of a development launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevLaunch {
    /// The `vibe-app-server` executable.
    pub server_bin: PathBuf,
    /// The `config.toml` template for the dev `VIBE_HOME`.
    pub home_template: PathBuf,
    /// The run folder holding `vibe-home/`, `home/` and `work/`.
    pub run_dir: PathBuf,
}

fn explicit(name: &'static str) -> Result<Option<PathBuf>, LaunchError> {
    match std::env::var_os(name) {
        None => Ok(None),
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(Some(path))
            } else {
                Err(LaunchError::NotAbsolute(name))
            }
        }
    }
}

fn require_absolute(path: &Path, name: &'static str) -> Result<(), LaunchError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(LaunchError::NotAbsolute(name))
    }
}

impl DevLaunch {
    /// Resolves the inputs from the environment, falling back to the repo layout under
    /// `repo_root`.
    ///
    /// # Errors
    /// [`LaunchError::NotAbsolute`] when a variable holds a relative path.
    pub fn from_env(repo_root: &Path) -> Result<Self, LaunchError> {
        let server_name = if cfg!(windows) {
            "vibe-app-server.exe"
        } else {
            "vibe-app-server"
        };
        let bin_dir = if cfg!(windows) { "Scripts" } else { "bin" };
        Ok(Self {
            server_bin: explicit("VW_DEV_SERVER_BIN")?
                .unwrap_or_else(|| repo_root.join("data/venv").join(bin_dir).join(server_name)),
            home_template: explicit("VW_DEV_HOME_TEMPLATE")?
                .unwrap_or_else(|| repo_root.join("data/vibe-home-template/config.toml")),
            run_dir: explicit("VW_DEV_RUN_DIR")?
                .unwrap_or_else(|| repo_root.join(".scratch/dev-app")),
        })
    }

    /// Prepares the run folder and builds the supervisor config: `VIBE_HOME` and `HOME` inside the
    /// run folder, a throwaway git identity, the server's working directory in `work/`.
    ///
    /// # Errors
    /// [`LaunchError::ServerMissing`], [`LaunchError::TemplateMissing`] or
    /// [`LaunchError::Prepare`].
    pub fn supervisor_config(&self) -> Result<SupervisorConfig, LaunchError> {
        require_absolute(&self.server_bin, "server_bin")?;
        require_absolute(&self.home_template, "home_template")?;
        require_absolute(&self.run_dir, "run_dir")?;
        if !self.server_bin.is_file() {
            return Err(LaunchError::ServerMissing(self.server_bin.clone()));
        }
        let vibe_home = self.run_dir.join("vibe-home");
        let home = self.run_dir.join("home");
        let work = self.run_dir.join("work");
        for dir in [&vibe_home, &home, &work] {
            std::fs::create_dir_all(dir).map_err(|source| LaunchError::Prepare {
                path: dir.clone(),
                source,
            })?;
        }
        let config = vibe_home.join("config.toml");
        if !config.exists() {
            if !self.home_template.is_file() {
                return Err(LaunchError::TemplateMissing(self.home_template.clone()));
            }
            std::fs::copy(&self.home_template, &config).map_err(|source| LaunchError::Prepare {
                path: config.clone(),
                source,
            })?;
        }
        crate::extension_paths::prepare(&config);
        log_title_model(&vibe_home);
        log_commit_signature(&vibe_home);
        clear_connector_cache(&vibe_home);
        let gitconfig = home.join(".gitconfig");
        if !gitconfig.exists() {
            std::fs::write(
                &gitconfig,
                "[user]\n\tname = Vibe Work Dev\n\temail = dev@example.invalid\n[init]\n\tdefaultBranch = main\n",
            )
            .map_err(|source| LaunchError::Prepare {
                path: gitconfig.clone(),
                source,
            })?;
        }
        let mut supervisor = SupervisorConfig::new(self.server_bin.clone(), vibe_home, work);
        supervisor.home = Some(home);
        supervisor.extra_env = vec![
            ("GIT_CONFIG_GLOBAL".to_owned(), gitconfig.into_os_string()),
            ("GIT_CONFIG_NOSYSTEM".to_owned(), OsString::from("1")),
            ("PYTHONUNBUFFERED".to_owned(), OsString::from("1")),
        ];
        // WORKAROUND(Windows without an interactive logon, e.g. over SSH): Vibe reads the
        // Credential Manager on `session/list` and fails with 1312 there; a dev run takes its key
        // from a supplied proxy placeholder, so the keyring is switched off (same as the live tests).
        if cfg!(windows) {
            supervisor.extra_env.push((
                "PYTHON_KEYRING_BACKEND".to_owned(),
                OsString::from("keyring.backends.null.Keyring"),
            ));
        }
        Ok(supervisor)
    }
}

/// How a user build starts `vibe-app-server`: Vibe Work's own `VIBE_HOME`
/// under the app's data folder, whose `.env` never holds a key; optionally the CLI's session folder
/// shared through `session_logging.save_dir`.
/// The key arrives only as the proxy placeholder (`vw_keys::KeyService`, started by the host).
/// A missing `config.toml` starts with Large 4; an existing config keeps the user's model choice.
///
/// Unlike [`DevLaunch`], it never sets `PYTHON_KEYRING_BACKEND`: Vibe keeps connector OAuth tokens
/// in the OS keyring, which a user build must keep (the null backend is for SSH test runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductLaunch {
    /// The `vibe-app-server` executable: Vibe installed at setup into the app's own folder, or the user's own
    /// install they chose (`vw_vibe_install`). It may not exist yet (`server_may_be_missing`).
    pub server_bin: PathBuf,
    /// Vibe is not installed yet: build the config anyway; the core starts the supervisor in `notInstalled`
    /// and launches the server when the install finishes.
    pub server_may_be_missing: bool,
    /// The app's data folder (`vibe-home/` and `work/` live under it).
    pub app_data: PathBuf,
    /// The Vibe CLI's session folder to share (`~/.vibe/logs/session`), or `None`.
    pub cli_session_dir: Option<PathBuf>,
}

impl ProductLaunch {
    /// The supervisor config. Removes a key line from our own `VIBE_HOME/.env` if one appeared
    /// there (the `.env` of Vibe Work's home must stay secret-free).
    ///
    /// # Errors
    /// [`LaunchError::ServerMissing`] (unless `server_may_be_missing`), [`LaunchError::Prepare`].
    pub fn supervisor_config(&self) -> Result<SupervisorConfig, LaunchError> {
        require_absolute(&self.server_bin, "server_bin")?;
        require_absolute(&self.app_data, "app_data")?;
        if let Some(dir) = &self.cli_session_dir {
            require_absolute(dir, "cli_session_dir")?;
        }
        if !self.server_may_be_missing && !self.server_bin.is_file() {
            return Err(LaunchError::ServerMissing(self.server_bin.clone()));
        }
        let vibe_home = self.app_data.join("vibe-home");
        let work = self.app_data.join("work");
        for dir in [&vibe_home, &work] {
            std::fs::create_dir_all(dir).map_err(|source| LaunchError::Prepare {
                path: dir.clone(),
                source,
            })?;
        }
        let dotenv = vibe_home.join(".env");
        if dotenv.is_file() {
            vw_keys::existing::remove_dotenv_key(&dotenv).map_err(|source| {
                LaunchError::Prepare {
                    path: dotenv.clone(),
                    source,
                }
            })?;
        }
        crate::default_model::ensure(&vibe_home).map_err(|source| LaunchError::Prepare {
            path: vibe_home.join("config.toml"),
            source,
        })?;
        crate::extension_paths::prepare(&vibe_home.join("config.toml"));
        log_title_model(&vibe_home);
        log_commit_signature(&vibe_home);
        clear_connector_cache(&vibe_home);
        let mut supervisor = SupervisorConfig::new(self.server_bin.clone(), vibe_home, work);
        supervisor.extra_env = vec![("PYTHONUNBUFFERED".to_owned(), OsString::from("1"))];
        if let Some(dir) = &self.cli_session_dir {
            supervisor.extra_env.push((
                "VIBE_SESSION_LOGGING__SAVE_DIR".to_owned(),
                dir.clone().into_os_string(),
            ));
        }
        Ok(supervisor)
    }
}

/// Vibe's connector catalog cache in its `VIBE_HOME` (Vibe 2.26.1 `core/paths/_vibe_home.py:23`).
pub const CONNECTOR_CACHE: &str = "connector_bootstrap_cache.json";

/// Removes Vibe's connector catalog cache before the server starts: Vibe then loads the catalog from
/// Mistral, through the model proxy, at least once per run, and the proxy learns the connector aliases its connector
/// rule needs from that answer (it never trusts this file, which the agent could write). Costs one catalog request
/// per app start. A missing file is fine; a failure is logged (the proxy then judges unknown connectors as forges).
fn clear_connector_cache(vibe_home: &Path) {
    match std::fs::remove_file(vibe_home.join(CONNECTOR_CACHE)) {
        Ok(()) => tracing::info!(
            "connectors: Vibe's catalog cache removed (reloaded through the model proxy)"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(%error, "connectors: Vibe's catalog cache could not be removed");
        }
    }
}

/// Turns the agent's commit credit line off by default in `vibe_home/config.toml`
/// ([`crate::commit_signature`]) and logs what happened. Never fails the launch.
fn log_commit_signature(vibe_home: &Path) {
    use crate::commit_signature::CommitSignature;
    match crate::commit_signature::ensure(vibe_home) {
        Ok(CommitSignature::Added) => {
            tracing::info!("agent commits: credit line off (include_commit_signature = false)");
        }
        Ok(CommitSignature::Kept(on)) => tracing::info!(
            on,
            "agent commits: config sets include_commit_signature, kept"
        ),
        Ok(CommitSignature::Skipped(why)) => {
            tracing::warn!(%why, "agent commits: config not changed");
        }
        Err(error) => tracing::warn!(%error, "agent commits: config.toml could not be updated"),
    }
}

/// Sets the session-title model in `vibe_home/config.toml` ([`crate::title_model`]) and logs what
/// happened. Never fails the launch: without it sessions on Mistral Large 4 only lose their automatic title.
fn log_title_model(vibe_home: &Path) {
    match crate::title_model::ensure(vibe_home) {
        Ok(outcome @ crate::title_model::TitleModel::Skipped(_)) => {
            tracing::warn!("{}", outcome.log_line());
        }
        Ok(outcome) => tracing::info!("{}", outcome.log_line()),
        Err(error) => tracing::warn!(%error, "session titles: config.toml could not be updated"),
    }
}
