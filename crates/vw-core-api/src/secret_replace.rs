//! Native user confirmation before replacing a stored secret or login password.
//!
//! Depends on: `vw-gate` confirmer and `vw-keys` metadata. Called by `secrets` and `secret_generate`.
//! The dialog receives only the entry name, never old or new values.

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer, Requester};

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// Native method for replacing a secret value, whether typed or generated.
pub(crate) const SECRET_METHOD: &str = "vibe-work/replaceVaultSecret";
/// Native method for replacing a login password, whether typed or generated.
pub(crate) const LOGIN_METHOD: &str = "vibe-work/replaceVaultLoginPassword";

/// Requires a native yes before any replacement write or generation/copy side effect.
pub(crate) async fn confirm(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    id: &str,
    login: bool,
) -> Result<(), CoreError> {
    let name = if login {
        keys.logins()
            .await
            .map_err(|e| crate::keys::key_error(&e))?
            .into_iter()
            .find(|item| item.id == id)
            .map(|item| item.name)
    } else {
        keys.project_secrets()
            .await
            .map_err(|e| crate::keys::key_error(&e))?
            .into_iter()
            .find(|item| item.id == id)
            .map(|item| item.name)
    }
    .ok_or_else(|| crate::keys::key_error(&vw_keys::KeyError::NoSuchSecret))?;
    let (method, kind) = if login {
        (LOGIN_METHOD, "login password")
    } else {
        (SECRET_METHOD, "secret value")
    };
    let request = ConfirmationRequest {
        method: method.to_owned(),
        requester: Requester::Window,
        summary: format!(
            "Replace the {kind} for {}? This deletes the old value and cannot be undone.",
            vw_gate::dialog_quote(&name)
        ),
        params: json!({}),
        always: None,
    };
    if shared.confirmer.confirm(&request).await != Confirmation::Approved {
        return Err(CoreError::new(
            ErrorCode::ConfirmationDeclined,
            "the vault entry was kept",
        ));
    }
    Ok(())
}
