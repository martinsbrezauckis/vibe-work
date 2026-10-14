//! Consent and install half of the update flow, a second `impl UpdateManager` kept apart so the
//! security property can be read in one place: **nothing reaches the backend's `install` except through
//! [`UpdateManager::install`], and that refuses without a recorded consent for the exact `ready` version.**
//!
//! - Main window: the Tauri command behind the "Install and restart" button records the consent (the click is
//!   the consent; the command is granted to the `main` webview only).
//! - Debug API: its request is the consent (the Debug API acts as the user;
//!   no native dialog is needed). The requester is kept for the log line.
//! - A consent is for one version, single use (taken by the attempt), and cleared by every state change.
//!
//! Depends on: [`crate::update_flow`] (state), [`crate::update_store`]. Called by: `app` (`commands.rs`), tests.

use crate::update_flow::Consent;
use crate::update_flow::{Phase, Stage, UpdateError, UpdateManager, UpdateRequester};
use crate::update_store::Pending;
use crate::update_view::Offer;

impl UpdateManager {
    /// Records the user's agreement to install the `ready` version: the main window's click or the Debug API's
    /// request.
    ///
    /// # Errors
    /// [`UpdateError::WrongState`] (nothing is `ready`), [`UpdateError::Busy`].
    pub fn consent(&self, requester: UpdateRequester) -> Result<(), UpdateError> {
        let version = self.ready_version()?;
        tracing::info!(?requester, %version, "update: install agreed");
        self.with(|s| match &s.phase {
            Phase::Ready { version: now, .. } if *now == version && !s.busy => {
                s.consent = Some(Consent { version });
                Ok(())
            }
            _ if s.busy => Err(UpdateError::Busy),
            _ => Err(UpdateError::WrongState),
        })
    }

    fn ready_version(&self) -> Result<String, UpdateError> {
        self.with(|s| match &s.phase {
            Phase::Ready { version, .. } if !s.busy => Ok(version.clone()),
            _ if s.busy => Err(UpdateError::Busy),
            _ => Err(UpdateError::WrongState),
        })
    }

    /// Installs the `ready` update and restarts, **only** with a recorded consent for that exact version.
    /// The consent is used up by the attempt.
    ///
    /// # Errors
    /// [`UpdateError::ConsentRequired`] without consent; [`UpdateError::WrongState`]; [`UpdateError::Busy`];
    /// [`UpdateError::Failed`] when the package is bad or the install fails (phase `failed`, offer kept).
    pub async fn install(&self) -> Result<(), UpdateError> {
        // The check and the take happen under one lock, together with the move to `installing`.
        let (offer, pending) = self.with(|s| {
            if s.busy {
                return Err(UpdateError::Busy);
            }
            let Phase::Ready { version, .. } = &s.phase else {
                return Err(UpdateError::WrongState);
            };
            let version = version.clone();
            let consent = s.consent.take();
            if consent.as_ref().map(|c| c.version.as_str()) != Some(version.as_str()) {
                return Err(UpdateError::ConsentRequired);
            }
            let (Some(offer), Some(pending)) = (s.offer.clone(), s.pending.clone()) else {
                return Err(UpdateError::WrongState);
            };
            s.busy = true;
            s.phase = Phase::Installing { version };
            Ok((offer, pending))
        })?;
        let result = self.install_inner(offer, pending).await;
        self.release();
        result
    }

    async fn install_inner(&self, offer: Offer, pending: Pending) -> Result<(), UpdateError> {
        let version = offer.version.clone();
        let store = self.0.store.clone();
        let pubkey = self.0.cfg.pubkey.clone();
        let read = tokio::task::spawn_blocking(move || store.read_verified(&pending, &pubkey))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(error) => {
                // The stored package is not trustworthy any more: drop it; "Try again" fetches a fresh one.
                self.0.store.clear();
                self.with(|s| {
                    s.pending = None;
                    s.offer = None;
                });
                self.fail(Stage::Download, error.clone(), Some(version));
                return Err(UpdateError::Failed(error));
            }
        };
        if let Err(error) = self.0.backend.install(offer, bytes).await {
            self.fail(Stage::Install, error.clone(), Some(version));
            return Err(UpdateError::Failed(error));
        }
        self.0.store.clear();
        self.with(|s| {
            s.pending = None;
            s.phase = Phase::Restarting {
                version: version.clone(),
            };
        });
        self.save_status("restarting", Some(&version), None);
        self.0.hooks.restart().await;
        Ok(())
    }

    /// Consent plus install: what a click on "Install and restart" does.
    ///
    /// # Errors
    /// Those of [`UpdateManager::consent`] and [`UpdateManager::install`].
    pub async fn request_install(&self, requester: UpdateRequester) -> Result<(), UpdateError> {
        self.consent(requester)?;
        self.install().await
    }
}
