//! Download half of the update flow: the explicit "Download" (ask-before-downloading mode, retry) and
//! the download itself (also started by a check in the automatic mode). The package is verified by the plugin
//! as it finishes and again by [`crate::update_verify`]; only then is it stored and the phase becomes `ready`.
//! A package that does not verify is deleted and never reaches `ready`.
//!
//! Depends on: [`crate::update_flow`] (state), [`crate::update_store`], [`crate::update_verify`].
//! Called by: [`crate::update_flow`] (after a check), `app` (`commands.rs`), tests.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::update_flow::{
    InstallMode, Offer, Phase, Progress, Stage, UpdateError, UpdateManager, UpdateNotice,
    UpdateRequester,
};
use crate::update_verify::verify_package;

impl UpdateManager {
    /// Starts the download the user asked for (the "ask before downloading" mode, or a retry), from the window or
    /// the Debug API alike.
    ///
    /// # Errors
    /// [`UpdateError::LinkOnly`], [`UpdateError::WrongState`], [`UpdateError::Busy`], or
    /// [`UpdateError::Failed`] with the download's failure (the phase is then `failed`).
    pub async fn download(&self, requester: UpdateRequester) -> Result<(), UpdateError> {
        if self.0.cfg.mode == InstallMode::LinkOnly {
            return Err(UpdateError::LinkOnly);
        }
        let offer = self.downloadable()?;
        tracing::info!(?requester, version = %offer.version, "update: download asked");
        self.acquire()?;
        // The state may have moved meanwhile.
        let offer = match self.downloadable() {
            Ok(offer) => offer,
            Err(error) => {
                self.release();
                return Err(error);
            }
        };
        self.download_inner(offer).await;
        self.release();
        match self.view().phase {
            Phase::Failed { error, .. } => Err(UpdateError::Failed(error)),
            _ => Ok(()),
        }
    }

    /// The offer that may be downloaded now: phase `available`, or `failed` at the download step.
    pub(crate) fn downloadable(&self) -> Result<Offer, UpdateError> {
        self.with(|s| match (&s.phase, &s.offer) {
            (
                Phase::Available { .. }
                | Phase::Failed {
                    stage: Stage::Download,
                    ..
                },
                Some(offer),
            ) => Ok(offer.clone()),
            _ => Err(UpdateError::WrongState),
        })
    }

    pub(crate) async fn download_inner(&self, offer: Offer) {
        let version = offer.version.clone();
        self.with(|s| {
            s.consent = None;
            s.phase = Phase::Downloading {
                version: version.clone(),
                notes: offer.notes.clone(),
                received: 0,
                total: None,
            };
        });
        let progress = {
            let this = self.clone();
            let last = Mutex::new(Instant::now());
            let notes = offer.notes.clone();
            let version = version.clone();
            Arc::new(move |received: u64, total: Option<u64>| {
                let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
                let done = total.is_some_and(|t| received >= t);
                if !done && last.elapsed() < Duration::from_millis(200) {
                    return;
                }
                *last = Instant::now();
                drop(last);
                this.with(|s| {
                    if matches!(s.phase, Phase::Downloading { .. }) {
                        s.phase = Phase::Downloading {
                            version: version.clone(),
                            notes: notes.clone(),
                            received,
                            total,
                        };
                    }
                });
            }) as Progress
        };
        let bytes = match self.0.backend.download(offer.clone(), progress).await {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail(Stage::Download, error, Some(version));
                return;
            }
        };
        if let Err(error) = verify_package(&bytes, &offer.signature, &self.0.cfg.pubkey, &version) {
            self.0.store.clear();
            self.fail(Stage::Download, error, Some(version));
            return;
        }
        let store = self.0.store.clone();
        let saved = {
            let offer = offer.clone();
            tokio::task::spawn_blocking(move || {
                store.save(
                    &offer.version,
                    offer.notes.as_deref(),
                    &offer.signature,
                    &bytes,
                )
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r)
        };
        match saved {
            Ok(pending) => {
                self.with(|s| {
                    s.pending = Some(pending);
                    s.phase = Phase::Ready {
                        version: version.clone(),
                        notes: offer.notes.clone(),
                    };
                });
                self.save_status("ready", Some(&version), None);
                self.notify_once(UpdateNotice::Ready { version });
            }
            Err(error) => self.fail(Stage::Download, error, Some(version)),
        }
    }
}
