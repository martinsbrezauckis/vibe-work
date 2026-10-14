//! The update flow: notify, download, then ask. One [`UpdateManager`] owns the state machine, the
//! downloaded package on disk and the user's consent; the network side and the OS are behind the traits of
//! [`crate::update_view`], so everything here runs without Tauri and is tested against a scripted backend.
//!
//! State machine:
//! `idle -> checking -> available -> downloading -> ready -> installing -> restarting`, plus `failed`.
//! Link-only installs stop at `available`. A verified package on disk starts the flow at `ready`.
//!
//! **Consent is a security property and is enforced here, not in the UI.** [`UpdateManager::install`] installs
//! only when a consent for exactly the current `ready` version is recorded; [`UpdateManager::consent`] records it
//! for the main window (the click) or the Debug API (its request: it acts as the user). A consent is
//! single use, bound to one version, and dies with any state change. There is no other path to the backend's `install`.
//!
//! Invariants:
//! - `ready` is reached only after our own signature check of the package ([`crate::update_verify`]); the package
//!   is verified again when it is read back for the install.
//! - The app never installs or restarts without that consent.
//! - Operations are exclusive: a second action while one runs gets [`UpdateError::Busy`].
//! - Locks are never held across an await.
//!
//! The consent and install half is in [`crate::update_consent`] (same type, a second `impl`).
//!
//! **Companion check.** The app can attach one more check ([`UpdateManager::set_companion_check`]: the
//! signed supported-Vibe manifest on the same channel). It runs inside [`UpdateManager::check`], concurrently with
//! the app's own check, and never on its own: there is no network beyond the update check.
//!
//! Depends on: tokio (`watch`), [`crate::settings`],
//! [`crate::updates`] (status file), [`crate::update_store`], [`crate::update_view`], [`crate::update_verify`].
//! Called by: `app` (`updater.rs`, `commands.rs`), the private tests.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::settings::HostSettings;
use crate::update_store::{Pending, UpdateStore};
pub use crate::update_view::{
    BoxFuture, InstallMode, OFFICIAL_RELEASE_TAGS, Offer, Phase, Progress, Stage, UpdateBackend,
    UpdateHooks, UpdateNotice, UpdateRequester, UpdateView, release_page_for,
};
use crate::updates::UpdateStatus;

/// Why an action was refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// Another update step is running.
    #[error("an update step is already running")]
    Busy,
    /// The action does not fit the current state (nothing to download / install).
    #[error("there is no update waiting for that")]
    WrongState,
    /// Installing needs the user's agreement first; none is recorded.
    #[error("installing needs your agreement first")]
    ConsentRequired,
    /// This installation updates through its package manager.
    #[error("this installation updates through its package manager: open the release page")]
    LinkOnly,
    /// A step failed; the text says why in plain words.
    #[error("{0}")]
    Failed(String),
}

/// What the manager needs to know about this build and installation.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    /// The running version.
    pub current: String,
    /// The channel's `pubkey` (base64 minisign public key, as in the Tauri config).
    pub pubkey: String,
    /// In-app or link-only.
    pub mode: InstallMode,
    /// The app's data folder (`updates/`, `updates.json`, `settings.json` live here).
    pub data_dir: PathBuf,
}

/// A recorded agreement to install one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Consent {
    pub(crate) version: String,
}

pub(crate) struct State {
    pub(crate) phase: Phase,
    pub(crate) offer: Option<Offer>,
    pub(crate) pending: Option<Pending>,
    pub(crate) consent: Option<Consent>,
    pub(crate) busy: bool,
    pub(crate) auto_download: bool,
    pub(crate) last_checked: Option<u64>,
    pub(crate) status: UpdateStatus,
}

/// A check that runs together with every app update check; its result is its own business.
pub type CompanionCheck = Arc<dyn Fn() -> BoxFuture<()> + Send + Sync>;

pub(crate) struct Inner {
    pub(crate) cfg: UpdateConfig,
    pub(crate) companion: Mutex<Option<CompanionCheck>>,
    pub(crate) backend: Arc<dyn UpdateBackend>,
    pub(crate) hooks: Arc<dyn UpdateHooks>,
    pub(crate) store: UpdateStore,
    pub(crate) state: Mutex<State>,
    pub(crate) tx: watch::Sender<UpdateView>,
}

/// The update flow of one running app. Cheap to clone.
#[derive(Clone)]
pub struct UpdateManager(pub(crate) Arc<Inner>);

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

impl UpdateManager {
    /// Creates the manager. A verified package left on disk by an earlier run starts the flow at `ready`.
    #[must_use]
    pub fn new(
        cfg: UpdateConfig,
        backend: Arc<dyn UpdateBackend>,
        hooks: Arc<dyn UpdateHooks>,
    ) -> Self {
        let store = UpdateStore::new(&cfg.data_dir);
        let settings = HostSettings::load(&cfg.data_dir.join("settings.json"));
        let mut status = std::fs::read(cfg.data_dir.join("updates.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<UpdateStatus>(&bytes).ok())
            .unwrap_or_else(|| UpdateStatus::new(&cfg.current));
        status.current.clone_from(&cfg.current);
        // Only a link-only install never downloads, so only an in-app install resumes a stored package.
        let pending = if cfg.mode == InstallMode::InApp {
            store.load(&cfg.pubkey, &cfg.current)
        } else {
            None
        };
        let (phase, offer) = match &pending {
            Some(p) => (
                Phase::Ready {
                    version: p.version.clone(),
                    notes: p.notes.clone(),
                },
                Some(Offer {
                    version: p.version.clone(),
                    notes: p.notes.clone(),
                    signature: p.signature.clone(),
                }),
            ),
            None => (Phase::Idle, None),
        };
        let state = State {
            phase,
            offer,
            pending,
            consent: None,
            busy: false,
            auto_download: settings.auto_download(),
            last_checked: None,
            status,
        };
        let view = Self::view_of(&cfg, &state);
        Self(Arc::new(Inner {
            cfg,
            companion: Mutex::new(None),
            backend,
            hooks,
            store,
            state: Mutex::new(state),
            tx: watch::channel(view).0,
        }))
    }

    fn view_of(cfg: &UpdateConfig, state: &State) -> UpdateView {
        UpdateView {
            current: cfg.current.clone(),
            mode: cfg.mode,
            auto_download: state.auto_download,
            release_page: state
                .offer
                .as_ref()
                .and_then(|o| release_page_for(&o.version)),
            last_checked: state.last_checked,
            phase: state.phase.clone(),
        }
    }

    /// Runs `f` on the state under the lock and publishes the resulting view if it changed.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        let result = f(&mut state);
        let view = Self::view_of(&self.0.cfg, &state);
        drop(state);
        self.0.tx.send_if_modified(|current| {
            if *current == view {
                false
            } else {
                *current = view;
                true
            }
        });
        result
    }

    /// The current view.
    #[must_use]
    pub fn view(&self) -> UpdateView {
        self.0.tx.borrow().clone()
    }

    /// A receiver that wakes on every view change (the first value is the current one).
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<UpdateView> {
        self.0.tx.subscribe()
    }

    /// Attaches the check that runs together with every [`UpdateManager::check`] (the supported-Vibe
    /// manifest). Replaces an earlier one.
    pub fn set_companion_check(&self, check: CompanionCheck) {
        *self
            .0
            .companion
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(check);
    }

    /// Runs one check after `delay` in the background.
    pub fn spawn_check(&self, runtime: &Handle, delay: Duration) {
        let this = self.clone();
        runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(error) = this.check().await {
                tracing::info!(%error, "update check not run");
            }
        });
    }

    pub(crate) fn acquire(&self) -> Result<(), UpdateError> {
        self.with(|s| {
            if s.busy {
                Err(UpdateError::Busy)
            } else {
                s.busy = true;
                Ok(())
            }
        })
    }

    pub(crate) fn release(&self) {
        self.with(|s| s.busy = false);
    }

    pub(crate) fn save_status(&self, outcome: &str, version: Option<&str>, error: Option<&str>) {
        let status = self.with(|s| {
            s.status.checked_at = now();
            outcome.clone_into(&mut s.status.outcome);
            s.status.version = version.map(str::to_owned);
            s.status.error = error.map(str::to_owned);
            s.status.clone()
        });
        if let Err(error) = status.save(&self.0.cfg.data_dir.join("updates.json")) {
            tracing::warn!(%error, "updates.json not written");
        }
    }

    /// Sends the OS notification once per kind and version (remembered in `updates.json`).
    pub(crate) fn notify_once(&self, notice: UpdateNotice) {
        let (first, status) = self.with(|s| {
            let (slot, version) = match &notice {
                UpdateNotice::Available { version, .. } => {
                    (&mut s.status.notified_available, version)
                }
                UpdateNotice::Ready { version } => (&mut s.status.notified_ready, version),
            };
            if slot.as_deref() == Some(version.as_str()) {
                (false, s.status.clone())
            } else {
                *slot = Some(version.clone());
                (true, s.status.clone())
            }
        });
        if first {
            let _ = status.save(&self.0.cfg.data_dir.join("updates.json"));
            self.0.hooks.notify(notice);
        }
    }

    /// Asks the server for a newer version and acts on the answer: link-only stops at `available`; in-app
    /// downloads at once when "download automatically" is on, otherwise waits at `available`. Returns when the
    /// download (if any) has finished.
    ///
    /// # Errors
    /// [`UpdateError::Busy`] when another step is running. A failed check is not an error: it becomes the
    /// `failed` phase.
    pub async fn check(&self) -> Result<(), UpdateError> {
        self.acquire()?;
        let companion = self
            .0
            .companion
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let also = async move {
            if let Some(check) = companion {
                check().await;
            }
        };
        let (result, ()) = tokio::join!(self.check_inner(), also);
        self.release();
        result
    }

    async fn check_inner(&self) -> Result<(), UpdateError> {
        let kept_ready = self.with(|s| {
            let ready = matches!(s.phase, Phase::Ready { .. });
            if !ready {
                s.phase = Phase::Checking;
            }
            ready
        });
        let answer = self.0.backend.check().await;
        let stamp = now();
        self.with(|s| s.last_checked = Some(stamp));
        match answer {
            Err(error) => {
                tracing::info!(%error, "update check failed");
                if kept_ready {
                    // Offline with a verified package on disk: the offer stays.
                    return Ok(());
                }
                self.fail(Stage::Check, error, None);
            }
            Ok(None) => {
                self.0.store.clear();
                self.with(|s| {
                    s.phase = Phase::Idle;
                    s.offer = None;
                    s.pending = None;
                    s.consent = None;
                });
                self.save_status("upToDate", None, None);
            }
            Ok(Some(offer)) => self.apply_offer(offer).await,
        }
        Ok(())
    }

    pub(crate) fn fail(&self, stage: Stage, error: String, version: Option<String>) {
        self.save_status("failed", version.as_deref(), Some(&error));
        self.with(|s| {
            s.consent = None;
            s.phase = Phase::Failed {
                stage,
                error,
                version,
            };
        });
    }

    async fn apply_offer(&self, offer: Offer) {
        if !is_newer(&offer.version, &self.0.cfg.current) {
            let error = format!(
                "the update server offered version {}, which is not newer than {}; ignored",
                offer.version, self.0.cfg.current
            );
            self.fail(Stage::Check, error, Some(offer.version));
            return;
        }
        let same_as_pending = self.with(|s| {
            s.pending
                .as_ref()
                .is_some_and(|p| p.version == offer.version && p.signature == offer.signature)
        });
        if same_as_pending {
            // The verified package on disk is this announcement: keep it (and re-bind the offer's details).
            let version = offer.version.clone();
            self.with(|s| {
                s.offer = Some(offer.clone());
                s.consent = None;
                s.phase = Phase::Ready {
                    version: offer.version.clone(),
                    notes: offer.notes.clone(),
                };
            });
            self.save_status("ready", Some(&version), None);
            return;
        }
        // A different (newer) announcement makes any stored package stale.
        self.0.store.clear();
        let (auto, mode) = self.with(|s| {
            s.pending = None;
            s.consent = None;
            s.offer = Some(offer.clone());
            s.phase = Phase::Available {
                version: offer.version.clone(),
                notes: offer.notes.clone(),
            };
            (s.auto_download, self.0.cfg.mode)
        });
        self.save_status("available", Some(&offer.version), None);
        let downloading = auto && mode == InstallMode::InApp;
        self.notify_once(UpdateNotice::Available {
            version: offer.version.clone(),
            downloading,
            link_only: mode == InstallMode::LinkOnly,
        });
        if downloading {
            self.download_inner(offer).await;
        }
    }

    /// Sets "Download updates automatically" and saves it in `settings.json`.
    ///
    /// # Errors
    /// The I/O error text when the file cannot be written (the setting then stays as it was).
    pub fn set_auto_download(&self, on: bool) -> Result<(), UpdateError> {
        let path = self.0.cfg.data_dir.join("settings.json");
        let mut settings = HostSettings::load(&path);
        settings.auto_download = Some(on);
        settings
            .save(&path)
            .map_err(|e| UpdateError::Failed(format!("cannot save the setting: {e}")))?;
        self.with(|s| s.auto_download = on);
        Ok(())
    }
}
