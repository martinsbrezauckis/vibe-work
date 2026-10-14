//! Generates vault values in the core and stores them without exposing them in a command result. The vendored
//! vault generator supplies the bytes; the key service owns the write and the platform clipboard owns timed copy.
//!
//! Depends on: `vw-vault`, `vw-keys`, `secrets` clipboard. Called by: `secrets::execute`.
//! Invariants: a new item refuses an existing name/site account as observed before storage; rotation preserves its id
//! and grants. Every answer carries metadata only, and a failed copy reports that the vault write succeeded.

use zeroize::Zeroizing;

use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::secrets_view::{PasswordOptions, SecretsRequest, SecretsResult};

/// Runs one user-only generation or rotation operation.
#[allow(
    clippy::too_many_lines,
    reason = "typed secret and login generation branches share one dispatcher"
)]
pub(crate) async fn execute(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    request: SecretsRequest,
) -> Result<SecretsResult, CoreError> {
    let map = |e: vw_keys::KeyError| crate::keys::key_error(&e);
    let result = match request {
        SecretsRequest::Generate {
            name,
            label,
            options,
        } => {
            let value = generate(options)?;
            let stored = keys
                .create_secret_labelled(
                    name,
                    vw_vault::SecretString::new(value.as_str()),
                    label.as_deref().and_then(vw_keys::SecretLabel::parse),
                )
                .await
                .map_err(map)?;
            copy_after_store(shared, value).await?;
            SecretsResult::Stored {
                secret: stored.secret.into(),
                replaced: stored.replaced,
            }
        }
        SecretsRequest::GenerateWithPermission {
            name,
            label,
            options,
            project,
            permission,
        } => {
            let project = crate::entry_permissions::user_project(shared, &project)?;
            let value = generate(options)?;
            let stored = keys
                .create_secret_labelled(
                    name,
                    vw_vault::SecretString::new(value.as_str()),
                    label.as_deref().and_then(vw_keys::SecretLabel::parse),
                )
                .await
                .map_err(map)?;
            if let Err(error) = keys
                .set_entry_permission(project.clone(), stored.secret.id.clone(), permission.into())
                .await
            {
                let _ = keys.remove_secret(stored.secret.id.clone()).await;
                return Err(map(error));
            }
            copy_after_store(shared, value).await?;
            let mut secret: crate::secrets_view::ProjectSecretView = stored.secret.into();
            secret.permission = Some(permission);
            secret.project_permission = Some(permission);
            secret
                .project_permissions
                .push(crate::secrets_view::ProjectPermissionView {
                    project,
                    permission,
                });
            SecretsResult::Stored {
                secret,
                replaced: stored.replaced,
            }
        }
        SecretsRequest::Regenerate { id, options } => {
            crate::secret_replace::confirm(shared, keys, &id, false).await?;
            let value = generate(options)?;
            keys.replace_secret(id, vw_vault::SecretString::new(value.as_str()))
                .await
                .map_err(map)?;
            copy_after_store(shared, value).await?;
            SecretsResult::Done {}
        }
        SecretsRequest::GenerateLogin {
            name,
            url,
            username,
            options,
        } => {
            let value = generate(options)?;
            let stored = keys
                .create_login(
                    name,
                    url,
                    username,
                    vw_vault::SecretString::exact(value.as_str()),
                )
                .await
                .map_err(map)?;
            copy_after_store(shared, value).await?;
            SecretsResult::LoginStored {
                login: stored.login.into(),
                replaced: stored.replaced,
            }
        }
        SecretsRequest::GenerateLoginWithPermission {
            name,
            url,
            username,
            options,
            project,
            permission,
        } => {
            let project = crate::entry_permissions::user_project(shared, &project)?;
            let value = generate(options)?;
            let stored = keys
                .create_login(
                    name,
                    url,
                    username,
                    vw_vault::SecretString::exact(value.as_str()),
                )
                .await
                .map_err(map)?;
            if let Err(error) = keys
                .set_entry_permission(project.clone(), stored.login.id.clone(), permission.into())
                .await
            {
                let _ = keys.remove_login(stored.login.id.clone()).await;
                return Err(map(error));
            }
            copy_after_store(shared, value).await?;
            let mut login: crate::secrets_view::LoginView = stored.login.into();
            login.permission = Some(permission);
            login.project_permission = Some(permission);
            login
                .project_permissions
                .push(crate::secrets_view::ProjectPermissionView {
                    project,
                    permission,
                });
            SecretsResult::LoginStored {
                login,
                replaced: stored.replaced,
            }
        }
        SecretsRequest::RegenerateLoginPassword { id, options } => {
            crate::secret_replace::confirm(shared, keys, &id, true).await?;
            let value = generate(options)?;
            keys.replace_login_password(id, vw_vault::SecretString::exact(value.as_str()))
                .await
                .map_err(map)?;
            copy_after_store(shared, value).await?;
            SecretsResult::Done {}
        }
        _ => {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "not a generation request",
            ));
        }
    };
    Ok(result)
}

/// Validates the requested size before calling the shared generator, which otherwise clamps it.
fn generate(options: PasswordOptions) -> Result<Zeroizing<String>, CoreError> {
    if !(8..=128).contains(&options.length) {
        return Err(CoreError::new(
            ErrorCode::InvalidParams,
            "password length must be 8 to 128",
        ));
    }
    Ok(Zeroizing::new(vw_vault::generate_password(
        options.length,
        options.uppercase,
        options.digits,
        options.symbols,
    )))
}

/// Reports a saved value accurately if the clipboard fails after the vault write.
async fn copy_after_store(shared: &Shared, value: Zeroizing<String>) -> Result<(), CoreError> {
    crate::secrets::clipboard(shared, value).await.map_err(|_| {
        shared.emit(CoreEvent::SecretsChanged {});
        CoreError::new(
            ErrorCode::Internal,
            "Value saved, but clipboard unavailable.",
        )
    })
}
