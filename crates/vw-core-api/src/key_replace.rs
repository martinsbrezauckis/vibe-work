//! Dispatches key commands and confirms replacement or selection before the model key changes.
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
    if let KeyRequest::UseMistralKey { item_id } = &request {
        confirm_switch(shared, keys, item_id).await?;
        refuse_busy(shared)?;
    } else {
        confirm(shared, &request).await?;
    }
    crate::keys::execute(keys, request).await
}

/// Ask before replacing a key in use unless its latest test rejected authentication.
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
    if keys.key_was_rejected() {
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

fn refuse_busy(shared: &Shared) -> Result<(), CoreError> {
    if shared
        .store
        .statuses()
        .iter()
        .any(|(id, _)| crate::recovery::working(shared, id))
    {
        let mut error = CoreError::new(
            ErrorCode::KeyService,
            "Wait until the chats finish, then switch keys.",
        );
        error.server_code = Some("chatsWorking".to_owned());
        return Err(error);
    }
    Ok(())
}

async fn confirm_switch(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    item_id: &str,
) -> Result<(), CoreError> {
    refuse_busy(shared)?;
    let rows = keys
        .mistral_keys()
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    let row = rows
        .iter()
        .find(|k| k.item_id == item_id)
        .ok_or_else(|| crate::keys::key_error(&vw_keys::KeyError::NotACandidate))?;
    if row.in_use {
        return Ok(());
    }
    let request = ConfirmationRequest {
        method: "vibe-work/useModelKey".to_owned(),
        requester: Requester::Window,
        summary: format!(
            "Use the key “{}” for Mistral? Chats continue with the new key.",
            row.label
        ),
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
