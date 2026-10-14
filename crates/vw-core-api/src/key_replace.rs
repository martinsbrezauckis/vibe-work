//! Dispatches key commands and confirms replacing an active model key before the store command writes it.
//! Depends on the key service and the core's native confirmer; called by the typed dispatcher.
//! Only a fixed warning crosses the dialog boundary, never the typed or stored key.

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};
use vw_keys::KeyPresence;

use crate::command::CommandResult;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::keys::KeyRequest;

/// Runs the existing handler after any required native replacement confirmation.
pub(crate) async fn execute(
    shared: &Shared,
    request: KeyRequest,
) -> Result<CommandResult, CoreError> {
    let Some(keys) = &shared.config.keys else {
        let mut error = CoreError::new(
            ErrorCode::KeyService,
            "this run has no key service (the key comes from the environment)",
        );
        error.server_code = Some("notConfigured".to_owned());
        return Err(error);
    };
    confirm(shared, &request).await?;
    crate::keys::execute(keys, request).await
}

/// Ask the native dialog only when a store request would replace a key currently in use.
async fn confirm(shared: &Shared, request: &KeyRequest) -> Result<(), CoreError> {
    if !matches!(request, KeyRequest::SetupStoreKey { .. }) {
        return Ok(());
    }
    let Some(keys) = &shared.config.keys else {
        return Ok(());
    };
    if !matches!(
        keys.status()
            .await
            .map_err(|e| crate::keys::key_error(&e))?
            .key,
        KeyPresence::Ready { .. }
    ) {
        return Ok(());
    }
    let request = ConfirmationRequest {
        method: "vibe-work/replaceModelKey".to_owned(),
        requester: Requester::Window,
        summary: "Replace the Mistral key in use? This replaces the key used by sessions with the new key. Agents never see either value.".to_owned(),
        params: json!({}),
        always: None,
    };
    if shared.confirmer.confirm(&request).await != Confirmation::Approved {
        return Err(CoreError::new(
            ErrorCode::ConfirmationDeclined,
            "The Mistral key was kept.",
        ));
    }
    Ok(())
}
