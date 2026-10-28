//! `vibe` commands: the Vibe install at setup, reachable the same way from the window and the Debug
//! API (the Debug API acts as the user; its request is the user's consent, like the window's click). Every
//! op answers the install view after it ran; progress arrives as [`crate::CoreEvent::VibeInstall`] events.
//!
//! Ops: `readStatus`, `prepare` (an offer: sources, checksums, commands, sizes), `confirm {offerId}` (starts the
//! install of exactly that offer), `cancel`, `checkExisting` (finds other installs, reads their versions),
//! `useExisting {path}`. Vibe updates: `prepareUpdate` (the consent offer for the update
//! the last verified supported-Vibe check found), `confirmUpdate {offerId}` (installs it next to the running Vibe;
//! it starts with Vibe Work's next start), `cancelUpdate`. The check itself runs only with the app's update check.
//! Errors: code `vibeInstall` with a reason word in `serverCode`: `wrongState`, `staleOffer`, `unsupported`,
//! `unknownExisting`, `unsupportedVersion`, `disk`, `noUpdate`, `notConfigured`.
//! No secret exists in an install, so nothing here needs redaction.
//!
//! Depends on: vw-vibe-install. Called by: `dispatch`.

use serde::{Deserialize, Serialize};
use vw_vibe_install::{InstallError, VibeInstaller};

use crate::command::{CommandResult, Origin};
use crate::error::{CoreError, ErrorCode};

/// One `vibe` operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum VibeRequest {
    /// The install view.
    ReadStatus {},
    /// A fresh offer to confirm (also the retry after a failure).
    Prepare {},
    /// The user agrees to the offer `offer_id`; the install starts.
    Confirm {
        /// The id from `prepare`'s offer.
        offer_id: String,
    },
    /// Cancels the running install, or withdraws the offer.
    Cancel {},
    /// Looks for Vibe installs already on this machine and reads their versions.
    CheckExisting {},
    /// Uses an install the last check found.
    UseExisting {
        /// Its path as the check listed it.
        path: String,
    },
    /// The consent offer for the available Vibe update.
    PrepareUpdate {},
    /// The user agrees to the update offer `offer_id`; it installs next to the running Vibe.
    ConfirmUpdate {
        /// The id from `prepareUpdate`'s offer.
        offer_id: String,
    },
    /// Cancels the running update install, or withdraws the update offer.
    CancelUpdate {},
}

fn refused(error: &InstallError) -> CoreError {
    let word = match error {
        InstallError::WrongState(_) => "wrongState",
        InstallError::StaleOffer => "staleOffer",
        InstallError::Pin(_) => "unsupported",
        InstallError::UnknownExisting(_) => "unknownExisting",
        InstallError::UnsupportedVersion(_) => "unsupportedVersion",
        InstallError::Disk(_) => "disk",
        InstallError::NoUpdate(_) => "noUpdate",
    };
    let mut core = CoreError::new(ErrorCode::VibeInstall, error.to_string());
    core.server_code = Some(word.to_owned());
    core
}

/// Runs `request` on the installer; answers the view after it.
///
/// # Errors
/// [`ErrorCode::VibeInstall`] with the reason word (module docs); `notConfigured` when this run has no installer
/// (development builds start their own server).
pub async fn execute(
    installer: Option<&VibeInstaller>,
    origin: Origin,
    request: VibeRequest,
) -> Result<CommandResult, CoreError> {
    let Some(installer) = installer else {
        let mut error = CoreError::new(
            ErrorCode::VibeInstall,
            "this run does not install Vibe (a development build starts its own server)",
        );
        error.server_code = Some("notConfigured".to_owned());
        return Err(error);
    };
    let requester = match origin {
        Origin::DebugApi => "debug-api",
        Origin::Ui => "window",
    };
    match request {
        VibeRequest::ReadStatus {} => {}
        VibeRequest::Prepare {} => {
            installer.prepare().map_err(|e| refused(&e))?;
        }
        VibeRequest::Confirm { offer_id } => {
            installer
                .confirm(&offer_id, requester)
                .map_err(|e| refused(&e))?;
        }
        VibeRequest::PrepareUpdate {} => {
            installer.prepare_update().map_err(|e| refused(&e))?;
        }
        VibeRequest::ConfirmUpdate { offer_id } => installer
            .confirm_update(&offer_id, requester)
            .map_err(|e| refused(&e))?,
        VibeRequest::CancelUpdate {} => installer.cancel_update().map_err(|e| refused(&e))?,
        VibeRequest::Cancel {} => installer.cancel().map_err(|e| refused(&e))?,
        VibeRequest::CheckExisting {} => {
            installer.check_existing().await;
        }
        VibeRequest::UseExisting { path } => installer
            .use_existing(&path)
            .await
            .map_err(|e| refused(&e))?,
    }
    Ok(CommandResult::VibeInstall {
        view: installer.view(),
    })
}
