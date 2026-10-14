//! Vibe updates, the second half of [`VibeInstaller`]: the check of the signed supported-Vibe manifest
//! and the consent-bound install of a newer tested Vibe next to the running one.
//!
//! - [`VibeInstaller::check_support`] runs together with the app's own update check (the app's update flow calls
//!   it; never on its own timer, so there is no network beyond that check). It fetches `vibe-support.json` and its
//!   `.sig` from the same channel as `latest.json`, verifies them with the build's updater key, refuses a rollback,
//!   and evaluates ([`crate::support`]). Any failure empties the offers (fail closed) and says why in `checkError`.
//! - [`VibeInstaller::prepare_update`] makes an offer (the same shape as the setup's, plus the requirements file's
//!   source and SHA-256) under a random id; [`VibeInstaller::confirm_update`] must name it (single use). The window's
//!   click and the Debug API's request are both the user's consent, as in setup.
//! - The install runs the setup's steps into `<root>/<new version>` (`crate::run`, [`crate::run::Slot::Update`]):
//!   the requirements file must have the signed SHA-256 before uv is even downloaded; `--require-hashes` then
//!   checks every package. The running Vibe keeps running; `active.json` makes the next start use the new one.
//! - Only Vibe Work's own install is ever updated. A user's own install gets a version notice, never an install.
//!
//! Depends on: [`crate::support`], [`crate::fetch`], [`crate::run`], [`crate::manager`]. Called by: vw-core-api
//! (`vibe` ops `prepareUpdate`, `confirmUpdate`, `cancelUpdate`), the app (`check_support` from the update flow),
//! the private tests.

use std::time::Duration;

use tokio::sync::watch;
use tracing::info;

use crate::layout::Layout;
use crate::manager::{InstallError, VibeInstaller};
use crate::pin::{Sources, UvAsset};
use crate::support::{
    MAX_MANIFEST_BYTES, MAX_SIGNATURE_BYTES, Manifest, Running, SIGNATURE_FILE, SeenSequence,
    SupportError, SupportedVibe,
};
use crate::view::{InstallPhase, InstallSource, SourceView, VibeUpdatePhase};

/// Longest the manifest check may take (both files).
pub const CHECK_LIMIT: Duration = Duration::from_secs(30);

/// Where the manifest is and what it is checked against; built by the app from its update channel config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportConfig {
    /// `…/vibe-support.json` on the release channel (same folder as `latest.json`; https, or loopback http in tests).
    pub manifest_url: String,
    /// The build's updater public key (base64 minisign text, the Tauri config's `pubkey`).
    pub pubkey: String,
    /// The running Vibe Work version (semver).
    pub app_version: String,
    /// This OS's Rust target triple ([`crate::this_target`]).
    pub target: String,
}

/// An installable update from the last verified check.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub(crate) entry: SupportedVibe,
    pub(crate) requirements_url: String,
    pub(crate) target: String,
}

/// The last check's result.
#[derive(Debug, Clone, Default)]
pub(crate) struct SupportState {
    pub(crate) checked_at: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) available: Option<Candidate>,
    pub(crate) upcoming: Option<String>,
    pub(crate) mismatch: Option<String>,
}

/// The offer waiting for its confirmation.
#[derive(Debug, Clone)]
pub(crate) struct PendingUpdate {
    pub(crate) offer_id: String,
    pub(crate) sources: Sources,
    pub(crate) requirements: UvAsset,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn sibling(manifest_url: &str, name: &str) -> Result<String, SupportError> {
    let base = url::Url::parse(manifest_url).map_err(|e| SupportError::Fetch(e.to_string()))?;
    base.join(name)
        .map(|u| u.to_string())
        .map_err(|e| SupportError::Fetch(e.to_string()))
}

/// Fetches and verifies the manifest (signature, file name, schema, rollback).
async fn fetch_verified(
    config: &SupportConfig,
    seen: &SeenSequence,
) -> Result<Manifest, SupportError> {
    let signature_url = sibling(&config.manifest_url, SIGNATURE_FILE)?;
    let fetch = |url: String, cap: u64| async move {
        crate::fetch::get_small(&url, cap, CHECK_LIMIT)
            .await
            .map_err(|e| SupportError::Fetch(e.to_string()))
    };
    let (manifest, signature) = tokio::join!(
        fetch(config.manifest_url.clone(), MAX_MANIFEST_BYTES),
        fetch(signature_url, MAX_SIGNATURE_BYTES)
    );
    let (manifest, signature) = (manifest?, signature?);
    let signature = String::from_utf8(signature)
        .map_err(|_| SupportError::Signature("the signature file is not text".into()))?;
    let manifest = crate::support::verify(&manifest, &signature, &config.pubkey)?;
    seen.accept(&manifest)?;
    Ok(manifest)
}

impl VibeInstaller {
    fn running(&self) -> Running {
        match &self.inner.lock().phase {
            InstallPhase::Installed {
                version,
                source: InstallSource::Private,
                ..
            } => Running::Private(version.clone()),
            InstallPhase::Installed {
                version,
                source: InstallSource::Existing,
                ..
            } => Running::Existing(version.clone()),
            _ => Running::None,
        }
    }

    /// Checks the supported-Vibe manifest on the release channel and updates the view's `update` part. Called by
    /// the app's update flow together with its own check. Never installs anything.
    ///
    /// # Errors
    /// The [`SupportError`] that gave no offer (also in the view as `checkError`).
    pub async fn check_support(&self, config: &SupportConfig) -> Result<(), SupportError> {
        let seen = SeenSequence::new(&self.inner.config.root);
        let outcome = fetch_verified(config, &seen).await;
        let running = self.running();
        let checked_at = Some(now());
        let mut state = self.inner.lock();
        let result = match outcome {
            Ok(manifest) => {
                let eval = crate::support::evaluate(
                    &manifest,
                    &config.app_version,
                    &config.target,
                    &running,
                    &self.inner.pinned,
                );
                // An update already installed in this run (starts next time) is not offered again.
                let installed = match &state.update {
                    VibeUpdatePhase::Installed { version } => Some(version.clone()),
                    _ => None,
                };
                let available = eval
                    .available
                    .filter(|e| {
                        installed.as_deref().is_none_or(|done| {
                            crate::support::compare_versions(&e.version, done)
                                == Some(std::cmp::Ordering::Greater)
                        })
                    })
                    .map(|entry| {
                        sibling(&config.manifest_url, &entry.requirements).map(|url| Candidate {
                            entry,
                            requirements_url: url,
                            target: config.target.clone(),
                        })
                    })
                    .transpose();
                match available {
                    Ok(available) => {
                        info!(
                            sequence = manifest.sequence,
                            available = ?available.as_ref().map(|c| &c.entry.version),
                            upcoming = ?eval.upcoming,
                            "supported-Vibe list checked"
                        );
                        state.support = crate::update::SupportState {
                            checked_at,
                            error: None,
                            available,
                            upcoming: eval.upcoming,
                            mismatch: eval.mismatch,
                        };
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        if let Err(error) = &result {
            info!(%error, "supported-Vibe list gave no offer");
            state.support = crate::update::SupportState {
                checked_at,
                error: Some(error.to_string()),
                ..Default::default()
            };
        }
        self.inner.publish(&state);
        result
    }

    /// Makes the consent offer for the available update (sources with checksums, commands, sizes).
    ///
    /// # Errors
    /// [`InstallError::NoUpdate`] without a verified offer or for the user's own install;
    /// [`InstallError::WrongState`] while an update installs; [`InstallError::Pin`].
    pub fn prepare_update(&self) -> Result<crate::view::InstallOffer, InstallError> {
        let base = self
            .inner
            .config
            .sources
            .clone()
            .map_err(InstallError::Pin)?;
        if let Running::Existing(version) = self.running() {
            return Err(InstallError::NoUpdate(format!(
                "Vibe Work uses your own Vibe {version} and never changes it: update it yourself"
            )));
        }
        let mut state = self.inner.lock();
        if matches!(state.update, VibeUpdatePhase::Installing { .. }) {
            return Err(InstallError::WrongState(
                "a Vibe update is installing".into(),
            ));
        }
        let candidate = state.support.available.clone().ok_or_else(|| {
            InstallError::NoUpdate("no Vibe update is available: check for updates first".into())
        })?;
        let entry = &candidate.entry;
        let layout = Layout::new(self.inner.config.root.clone(), &entry.version)
            .ok_or_else(|| InstallError::NoUpdate("the update's version is not plain".into()))?;
        if layout.installed().is_some() {
            crate::layout::save_active(&layout.root, &entry.version)
                .map_err(|e| InstallError::Disk(e.to_string()))?;
            state.update = VibeUpdatePhase::Installed {
                version: entry.version.clone(),
            };
            state.support.available = None;
            self.inner.publish(&state);
            return Err(InstallError::NoUpdate(format!(
                "Vibe {} is already installed: it starts the next time you open Vibe Work",
                entry.version
            )));
        }
        let wheel = entry
            .wheels
            .get(&candidate.target)
            .cloned()
            .ok_or_else(|| {
                InstallError::NoUpdate("this update has no Vibe build for this system".into())
            })?;
        let sources = Sources {
            vibe_version: entry.version.clone(),
            python: entry.python.clone(),
            pypi_page: format!("https://pypi.org/project/mistral-vibe/{}/", entry.version),
            requirements: String::new(),
            requirements_sha256: entry.requirements_sha256.clone(),
            packages: entry.packages,
            wheel: (wheel.filename, wheel.bytes, wheel.sha256),
            ..base
        };
        let requirements = UvAsset {
            url: candidate.requirements_url.clone(),
            bytes: entry.requirements_bytes,
            sha256: entry.requirements_sha256.clone(),
            archive: "txt".to_owned(),
        };
        let mut offer = crate::manager::offer(&sources, &layout);
        offer.sources.insert(
            0,
            SourceView {
                name: format!(
                    "Hash-locked requirements for Vibe {} (Vibe Work's signed release channel)",
                    entry.version
                ),
                url: requirements.url.clone(),
                sha256: Some(requirements.sha256.clone()),
                bytes: Some(requirements.bytes),
            },
        );
        offer.download_bytes += requirements.bytes;
        state.update_job = Some(PendingUpdate {
            offer_id: offer.offer_id.clone(),
            sources,
            requirements,
        });
        state.update = VibeUpdatePhase::Offered {
            offer: offer.clone(),
        };
        self.inner.publish(&state);
        Ok(offer)
    }

    /// The user's agreement to the update offer `offer_id`; installs it in the background next to the running Vibe.
    ///
    /// # Errors
    /// [`InstallError::StaleOffer`] when `offer_id` is not the current update offer; [`InstallError::WrongState`].
    pub fn confirm_update(&self, offer_id: &str, requester: &str) -> Result<(), InstallError> {
        let (job, cancel_rx) = {
            let mut state = self.inner.lock();
            match &state.update {
                VibeUpdatePhase::Offered { offer } if offer.offer_id == offer_id => {}
                VibeUpdatePhase::Offered { .. } => return Err(InstallError::StaleOffer),
                _ => {
                    return Err(InstallError::WrongState(
                        "there is no Vibe update offer to confirm".into(),
                    ));
                }
            }
            let pending = state
                .update_job
                .take()
                .filter(|p| p.offer_id == offer_id)
                .ok_or(InstallError::StaleOffer)?;
            let layout = Layout::new(
                self.inner.config.root.clone(),
                &pending.sources.vibe_version,
            )
            .ok_or_else(|| InstallError::NoUpdate("the update's version is not plain".into()))?;
            let (cancel_tx, cancel_rx) = watch::channel(false);
            state.cancel = Some(cancel_tx);
            state.update = crate::run::update_installing(
                "Download the requirements and uv",
                1,
                0,
                String::new(),
            );
            self.inner.publish(&state);
            let job = crate::run::Job {
                sources: pending.sources,
                layout,
                slot: crate::run::Slot::Update,
                requirements: Some(pending.requirements),
            };
            (job, cancel_rx)
        };
        info!(requester, version = %job.sources.vibe_version, "Vibe update agreed");
        tokio::spawn(crate::run::install(self.inner.clone(), job, cancel_rx));
        Ok(())
    }

    /// Cancels a running update install (processes killed, its folder removed) or withdraws the update offer.
    ///
    /// # Errors
    /// [`InstallError::WrongState`] when there is nothing to cancel.
    pub fn cancel_update(&self) -> Result<(), InstallError> {
        let mut state = self.inner.lock();
        match &state.update {
            VibeUpdatePhase::Installing { .. } => {
                if let Some(cancel) = &state.cancel {
                    cancel.send_replace(true);
                }
                Ok(())
            }
            VibeUpdatePhase::Offered { .. } => {
                state.update = VibeUpdatePhase::Idle {};
                state.update_job = None;
                self.inner.publish(&state);
                Ok(())
            }
            _ => Err(InstallError::WrongState("no Vibe update to cancel".into())),
        }
    }
}
