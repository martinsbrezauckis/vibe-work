//! Debug API bearer storage in an ordinary vault secret. The host/server call this Rust-only API;
//! chats use the existing secret broker and entry permissions. No token enters settings or events.

use vw_vault::{SecretString, items::VaultResource, project_secret};
use zeroize::Zeroizing;

use crate::{
    KeyError, KeyService,
    service::{blocking, now_ms},
};

/// Stable identity of the app's ordinary, agent-grantable vault item.
pub const DEBUG_TOKEN_ITEM: &str = "vibe-work-secret-deb06a9100000001";

impl KeyService {
    /// Loads or creates the Debug API token; `rotate` replaces its value while retaining permissions.
    /// Only the host/server may read this value directly. Agent use goes through `secret_run`.
    ///
    /// # Errors
    /// A locked or missing vault, storage failures, or unavailable OS randomness. Never falls back to a file.
    pub async fn debug_api_token(&self, rotate: bool) -> Result<SecretString, KeyError> {
        let _selection = self.selection.lock().await;
        let vault = self.vault();
        blocking(move || {
            let existing = vault.read_item(DEBUG_TOKEN_ITEM)?;
            if !rotate && let Some(item) = &existing {
                let resource = VaultResource::from_item(item).ok_or(KeyError::Internal)?;
                let raw = resource
                    .secret_fields
                    .get(project_secret::VALUE_FIELD)
                    .ok_or(KeyError::Internal)?;
                if raw.len() == 64
                    && raw
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    let token = SecretString::new(raw.clone());
                    token.register_as(vw_secrets::SecretKind::DebugApiToken);
                    return Ok(token);
                }
                return Err(KeyError::InvalidSecret(
                    "regenerate the Debug API token in Settings",
                ));
            }
            let mut bytes = Zeroizing::new([0_u8; 32]);
            getrandom::fill(bytes.as_mut()).map_err(|_| KeyError::Internal)?;
            let token = SecretString::new(hex::encode(bytes.as_ref()));
            let now = now_ms();
            if existing.is_some() {
                vault
                    .update_item(DEBUG_TOKEN_ITEM, |current| {
                        let mut resource = VaultResource::from_item(&current?)?;
                        resource.secret_fields.insert(
                            project_secret::VALUE_FIELD.to_owned(),
                            token.expose().to_owned(),
                        );
                        resource.updated_ms = now;
                        resource.to_item().ok()
                    })?
                    .ok_or(KeyError::NoSuchSecret)?;
            } else {
                let resource = project_secret::resource(
                    DEBUG_TOKEN_ITEM,
                    "Debug API token",
                    &token,
                    None,
                    None,
                    None,
                    now,
                );
                vault.upsert_item(&resource.to_item()?)?;
            }
            token.register_as(vw_secrets::SecretKind::DebugApiToken);
            token.register_as(vw_secrets::SecretKind::VaultValue);
            Ok(token)
        })
        .await?
    }
}
