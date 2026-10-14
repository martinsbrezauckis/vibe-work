//! The install's state machine: offer, consent, install, start; or the user's existing install.
//!
//! Consent (ShellX `agent_cli_setup.rs` pattern, rules): `prepare` makes an offer with a random id that
//! lists the official sources, the SHA-256 of what we download ourselves, the commands that will run, and the
//! sizes; nothing is downloaded before `confirm` names that exact id (single use: a new `prepare`, a `cancel` or
//! any state change makes it stale). The window's click and the Debug API's request are both the user's consent
//! (the Debug API acts as the user); the requester is logged.
//!
//! Phases: `notInstalled` → `offered` → `installing` → `installed` | `failed` (→ `prepare` again = retry).
//! `useExisting` goes from `notInstalled`/`offered`/`failed` straight to `installed` (source `existing`).
//! When an install finishes, the `on_ready` hook starts Vibe (the core's `Supervisor::launch_installed`); a
//! start failure is phase `failed` with kind `start`.
//!
//! Vibe updates are a second impl in [`crate::update`]: they never change the main phase; at start a
//! finished update (`active.json`, not older than the pin) is the install that runs, and the folder it replaced is
//! removed.
//!
//! Depends on: [`crate::run`] (the install task), [`crate::existing`], [`crate::layout`], [`crate::pin`],
//! tokio. Called by: vw-core-api (`vibe` commands), the app (resolving the server at start), private tests.
//! Invariants: at most one install task at a time; every phase change is published on the view channel.

use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tracing::info;
use vw_supervisor::Discovery;

use crate::layout::{Choice, Installed, Layout};
use crate::pin::{PinError, Sources};
use crate::view::{
    ExistingView, InstallOffer, InstallPhase, InstallSource, InstallView, SourceView,
    VibeUpdatePhase, VibeUpdateView,
};

/// Starts Vibe from the installed server; `Err` is the reason it did not start.
pub type OnReady =
    Arc<dyn Fn(PathBuf) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

/// Time limits of the steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The uv download.
    pub download: Duration,
    /// `uv python install`.
    pub python: Duration,
    /// `uv venv`.
    pub venv: Duration,
    /// `uv pip install` of Vibe and its dependencies.
    pub packages: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            download: Duration::from_secs(5 * 60),
            python: Duration::from_secs(10 * 60),
            venv: Duration::from_secs(2 * 60),
            packages: Duration::from_secs(20 * 60),
        }
    }
}

/// Everything the installer needs.
pub struct InstallerConfig {
    /// `<app data>/vibe`.
    pub root: PathBuf,
    /// What to install ([`Sources::pinned`] in the product), or why this build cannot install.
    pub sources: Result<Sources, PinError>,
    /// The environment the steps' allowlist is taken from (the app's own environment).
    pub parent_env: Vec<(OsString, OsString)>,
    /// Extra environment for the steps after the allowlist (tests: the fake uv's settings; empty in the product).
    pub extra_env: Vec<(String, OsString)>,
    /// Where existing installs are looked for.
    pub discovery: Discovery,
    /// Step time limits.
    pub limits: Limits,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// Not possible in the current phase (e.g. `confirm` with no offer, `prepare` while installing).
    #[error("not possible now: {0}")]
    WrongState(String),
    /// `confirm` named another offer than the current one.
    #[error(
        "the confirmation is for another offer (it changed or was already used): prepare again"
    )]
    StaleOffer,
    /// This build cannot install Vibe here.
    #[error("{0}")]
    Pin(PinError),
    /// `useExisting` named a path that the last check did not find, or that did not answer.
    #[error("{0} was not found by the last check (or did not answer as Vibe): check again")]
    UnknownExisting(String),
    /// `useExisting` of another version needs `acceptWarning: true` after the user saw the warning.
    #[error("{0}")]
    NeedsAcceptance(String),
    /// No Vibe update can be offered now (no verified offer, the user's own install, an update running).
    #[error("{0}")]
    NoUpdate(String),
    /// Writing the choice failed.
    #[error("cannot save the choice: {0}")]
    Disk(String),
}

pub(crate) struct State {
    pub(crate) phase: InstallPhase,
    pub(crate) existing: Vec<ExistingView>,
    pub(crate) checked_existing: bool,
    pub(crate) cancel: Option<watch::Sender<bool>>,
    /// The Vibe update install.
    pub(crate) update: VibeUpdatePhase,
    /// The last manifest check's result.
    pub(crate) support: crate::update::SupportState,
    /// The update offer's install data, until it is confirmed or replaced.
    pub(crate) update_job: Option<crate::update::PendingUpdate>,
}

pub(crate) struct Inner {
    pub(crate) config: InstallerConfig,
    pub(crate) layout: Option<Layout>,
    pub(crate) pinned: String,
    pub(crate) state: Mutex<State>,
    pub(crate) view: watch::Sender<InstallView>,
    pub(crate) on_ready: Mutex<Option<OnReady>>,
}

/// The installer. Cheap to clone (shared state).
#[derive(Clone)]
pub struct VibeInstaller {
    pub(crate) inner: Arc<Inner>,
}

impl VibeInstaller {
    /// Reads what is already there: a finished private install, or a saved existing-install choice (which wins:
    /// the user chose it). Otherwise `notInstalled`.
    #[must_use]
    pub fn new(config: InstallerConfig) -> Self {
        let pinned = match &config.sources {
            Ok(sources) => sources.vibe_version.clone(),
            Err(_) => Sources::pinned_version().unwrap_or_default(),
        };
        let layout = Layout::new(config.root.clone(), &pinned);
        let phase = layout
            .as_ref()
            .map_or(InstallPhase::NotInstalled {}, |layout| {
                if let Some(choice) = layout.choice() {
                    let warning = (choice.version != pinned)
                        .then(|| crate::existing::version_warning(&choice.version, &pinned));
                    InstallPhase::Installed {
                        version: choice.version,
                        server: choice.path.display().to_string(),
                        source: InstallSource::Existing,
                        disk_bytes: None,
                        install_ms: None,
                        warning,
                    }
                } else if let Some(done) = current_private(layout) {
                    InstallPhase::Installed {
                        version: done.version,
                        server: done.server.display().to_string(),
                        source: InstallSource::Private,
                        disk_bytes: Some(done.disk_bytes),
                        install_ms: None,
                        warning: None,
                    }
                } else {
                    InstallPhase::NotInstalled {}
                }
            });
        let state = State {
            phase,
            existing: Vec::new(),
            checked_existing: false,
            cancel: None,
            update: VibeUpdatePhase::Idle {},
            support: crate::update::SupportState::default(),
            update_job: None,
        };
        let view = watch::Sender::new(snapshot(&pinned, &state));
        Self {
            inner: Arc::new(Inner {
                config,
                layout,
                pinned,
                state: Mutex::new(state),
                view,
                on_ready: Mutex::new(None),
            }),
        }
    }

    /// Sets the hook that starts Vibe once it is installed (or chosen).
    pub fn set_on_ready(&self, hook: OnReady) {
        *self
            .inner
            .on_ready
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// The server to start now, if Vibe is installed or an existing install was chosen.
    #[must_use]
    pub fn server(&self) -> Option<PathBuf> {
        match &self.inner.lock().phase {
            InstallPhase::Installed { server, .. } => Some(PathBuf::from(server)),
            _ => None,
        }
    }

    /// Where a private install puts the server (it may not exist yet): the supervisor's configured program before
    /// the install. `None` when the pinned version is not a plain version (a build defect).
    #[must_use]
    pub fn expected_server(&self) -> Option<PathBuf> {
        self.inner.layout.as_ref().map(Layout::server)
    }

    /// The current view.
    #[must_use]
    pub fn view(&self) -> InstallView {
        self.inner.view.borrow().clone()
    }

    /// Every view change, current one first.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<InstallView> {
        self.inner.view.subscribe()
    }

    /// Makes a fresh offer (also the "retry" after a failure).
    ///
    /// # Errors
    /// [`InstallError::WrongState`] while installing or when installed; [`InstallError::Pin`].
    pub fn prepare(&self) -> Result<InstallOffer, InstallError> {
        let sources = self
            .inner
            .config
            .sources
            .clone()
            .map_err(InstallError::Pin)?;
        let layout = self.layout()?;
        let mut state = self.inner.lock();
        match &state.phase {
            InstallPhase::Installing { .. } => {
                return Err(InstallError::WrongState("an install is running".into()));
            }
            InstallPhase::Installed { .. } => {
                return Err(InstallError::WrongState("Vibe is already set up".into()));
            }
            _ => {}
        }
        let offer = offer(&sources, &layout);
        state.phase = InstallPhase::Offered {
            offer: offer.clone(),
        };
        self.inner.publish(&state);
        Ok(offer)
    }

    /// The user's agreement to the offer `offer_id`; starts the install in the background.
    ///
    /// # Errors
    /// [`InstallError::StaleOffer`] when `offer_id` is not the current offer; [`InstallError::WrongState`].
    pub fn confirm(&self, offer_id: &str, requester: &str) -> Result<(), InstallError> {
        let sources = self
            .inner
            .config
            .sources
            .clone()
            .map_err(InstallError::Pin)?;
        let layout = self.layout()?;
        let cancel_rx = {
            let mut state = self.inner.lock();
            match &state.phase {
                InstallPhase::Offered { offer } if offer.offer_id == offer_id => {}
                InstallPhase::Offered { .. } => return Err(InstallError::StaleOffer),
                _ => {
                    return Err(InstallError::WrongState(
                        "there is no offer to confirm".into(),
                    ));
                }
            }
            let (cancel_tx, cancel_rx) = watch::channel(false);
            state.cancel = Some(cancel_tx);
            state.phase = crate::run::installing("Download uv", 1, 0, String::new());
            self.inner.publish(&state);
            cancel_rx
        };
        info!(requester, version = %sources.vibe_version, "Vibe install agreed");
        let job = crate::run::Job {
            sources,
            layout,
            slot: crate::run::Slot::Setup,
            requirements: None,
        };
        tokio::spawn(crate::run::install(self.inner.clone(), job, cancel_rx));
        Ok(())
    }

    /// Cancels a running install (its processes are killed, the unfinished folder removed), or withdraws an offer.
    ///
    /// # Errors
    /// [`InstallError::WrongState`] when there is nothing to cancel.
    pub fn cancel(&self) -> Result<(), InstallError> {
        let mut state = self.inner.lock();
        match &state.phase {
            InstallPhase::Installing { .. } => {
                if let Some(cancel) = &state.cancel {
                    cancel.send_replace(true);
                }
                Ok(())
            }
            InstallPhase::Offered { .. } => {
                state.phase = InstallPhase::NotInstalled {};
                self.inner.publish(&state);
                Ok(())
            }
            _ => Err(InstallError::WrongState("nothing to cancel".into())),
        }
    }

    /// Looks for existing installs and reads their versions (a few seconds per candidate).
    pub async fn check_existing(&self) -> Vec<ExistingView> {
        let Ok(layout) = self.layout() else {
            return Vec::new();
        };
        let found = crate::existing::find(
            &self.inner.config.discovery,
            &layout.probe_dir(),
            &self.inner.config.parent_env,
            &self.inner.pinned,
            &layout.root,
        )
        .await;
        let _ = std::fs::remove_dir_all(layout.probe_dir());
        let mut state = self.inner.lock();
        state.existing.clone_from(&found);
        state.checked_existing = true;
        self.inner.publish(&state);
        found
    }

    /// Uses an existing install the last check found. Another version needs `accept_warning` (the user saw the
    /// warning). Starts Vibe from it.
    ///
    /// # Errors
    /// [`InstallError::UnknownExisting`], [`InstallError::NeedsAcceptance`], [`InstallError::WrongState`],
    /// [`InstallError::Disk`].
    pub async fn use_existing(&self, path: &str, accept_warning: bool) -> Result<(), InstallError> {
        let layout = self.layout()?;
        let (choice, warning) = {
            let state = self.inner.lock();
            if matches!(
                state.phase,
                InstallPhase::Installing { .. } | InstallPhase::Installed { .. }
            ) {
                return Err(InstallError::WrongState(
                    "Vibe is being installed or is already set up".into(),
                ));
            }
            let found = state
                .existing
                .iter()
                .find(|e| e.path == path && e.version.is_some())
                .ok_or_else(|| InstallError::UnknownExisting(path.to_owned()))?;
            if let (Some(warning), false) = (&found.warning, accept_warning) {
                return Err(InstallError::NeedsAcceptance(warning.clone()));
            }
            let choice = Choice {
                path: PathBuf::from(&found.path),
                version: found.version.clone().unwrap_or_default(),
            };
            (choice, found.warning.clone())
        };
        layout
            .save_choice(&choice)
            .map_err(|e| InstallError::Disk(e.to_string()))?;
        info!(path = %choice.path.display(), version = %choice.version, "existing Vibe chosen");
        let server = choice.path.clone();
        let installed = InstallPhase::Installed {
            version: choice.version,
            server: server.display().to_string(),
            source: InstallSource::Existing,
            disk_bytes: None,
            install_ms: None,
            warning,
        };
        crate::run::start(&self.inner, server, installed, "Start Vibe", 1).await;
        Ok(())
    }

    pub(crate) fn layout(&self) -> Result<Layout, InstallError> {
        self.inner.layout.clone().ok_or_else(|| {
            InstallError::WrongState("the pinned Vibe version is not a plain version".into())
        })
    }
}

impl Inner {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn publish(&self, state: &State) {
        self.view.send_replace(snapshot(&self.pinned, state));
    }

    pub(crate) fn on_ready(&self) -> Option<OnReady> {
        self.on_ready
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn snapshot(pinned: &str, state: &State) -> InstallView {
    InstallView {
        pinned_version: pinned.to_owned(),
        phase: state.phase.clone(),
        existing: state.existing.clone(),
        checked_existing: state.checked_existing,
        update: Box::new(VibeUpdateView {
            available: state
                .support
                .available
                .as_ref()
                .map(|c| c.entry.version.clone()),
            upcoming: state.support.upcoming.clone(),
            mismatch: state.support.mismatch.clone(),
            checked_at: state.support.checked_at,
            check_error: state.support.error.clone(),
            phase: state.update.clone(),
        }),
    }
}

/// The private install that runs at start: a finished update named by `active.json` when it is not
/// older than the pin, else the pinned install. The folder of any other version is removed then (the update it
/// was replaced by is the one starting; or a newer pin replaced an older update).
fn current_private(pinned: &Layout) -> Option<Installed> {
    let root = &pinned.root;
    let updated = crate::layout::active_version(root)
        .filter(|v| {
            crate::support::compare_versions(v, &pinned.version)
                .is_some_and(|o| o != std::cmp::Ordering::Less)
        })
        .and_then(|v| Layout::new(root.clone(), &v))
        .and_then(|l| l.installed());
    let chosen = updated.or_else(|| pinned.installed())?;
    crate::layout::remove_other_versions(root, &chosen.version);
    Some(chosen)
}

/// Download estimate besides uv and the Vibe wheel: a standalone `CPython` (~30 MB) and the dependencies' wheels
/// (~40 MB), measured on Linux.
const OTHER_DOWNLOAD_BYTES: u64 = 70 * 1024 * 1024;
/// Disk estimate after the install (Linux: venv 318 MB + Python 104 MB + uv 38 MB).
const DISK_BYTES: u64 = 470 * 1024 * 1024;

pub(crate) fn offer(sources: &Sources, layout: &Layout) -> InstallOffer {
    let steps = crate::run::steps(sources, layout, &Limits::default());
    let folder = layout.install_dir();
    InstallOffer {
        offer_id: random_id(),
        vibe_version: sources.vibe_version.clone(),
        python: sources.python.clone(),
        folder: folder.display().to_string(),
        sources: vec![
            SourceView {
                name: format!("uv {} (Astral)", sources.uv_version),
                url: sources.uv.url.clone(),
                sha256: Some(sources.uv.sha256.clone()),
                bytes: Some(sources.uv.bytes),
            },
            SourceView {
                name: format!("Python {} (installed by uv)", sources.python),
                url: "https://github.com/astral-sh/python-build-standalone/releases".to_owned(),
                sha256: None,
                bytes: None,
            },
            SourceView {
                name: format!(
                    "mistral-vibe {} (PyPI) and {} pinned packages",
                    sources.vibe_version,
                    sources.packages - 1
                ),
                url: sources.pypi_page.clone(),
                sha256: Some(sources.wheel.2.clone()),
                bytes: Some(sources.wheel.1),
            },
        ],
        steps: steps.iter().map(|s| s.display(&folder)).collect(),
        requirements_sha256: sources.requirements_sha256.clone(),
        packages: sources.packages,
        download_bytes: sources.uv.bytes + sources.wheel.1 + OTHER_DOWNLOAD_BYTES,
        disk_bytes: DISK_BYTES,
        docs_url: sources.docs.clone(),
    }
}

/// 128 random bits from the OS's random source, as hex (the offer id). The offer is not a secret (it is shown
/// on screen); the id only has to be unguessable before it is shown, so a stale or forged confirmation fails.
fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS random source: the time still makes each offer distinct (consent stays bound to the shown offer).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    crate::pin::hex(&bytes)
}
