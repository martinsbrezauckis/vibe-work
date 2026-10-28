//! The `secrets` command: Settings > Project
//! secrets and the Debug API list metadata; the window adds, renames, replaces, removes and copies secrets and
//! website logins, generates values, switches agent access, and revokes MCP and fill grants. Rename and
//! replace retain the item id; copy sends a listed value to the native clipboard. The Debug API may only
//! `list` (metadata, when agent access is open); all mutations and
//! copies are refused to it by `vault_scope`.
//!
//! Depends on: vw-keys (project secrets), vw-platform (native clipboard), `secret_uses`, `secrets_view`, `keys`
//! (error mapping). Called by: `dispatch` and `Core::start` (clipboard lifetime).
//! Invariants: no result carries a value; `add`'s value is registered as a stored secret by vw-keys before it is
//! stored; a change emits `secretsChanged`.

use crate::core::{CoreConfig, Shared};
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::secrets_view::{SecretsRequest, SecretsResult};
use zeroize::Zeroizing;

/// Installs the core-owned native clipboard for vault copy commands, preserving a supplied test service.
pub(crate) fn configure_clipboard(
    mut config: CoreConfig,
) -> Result<CoreConfig, crate::error::CoreStartError> {
    if config.clipboard.is_none() {
        config.clipboard = Some(std::sync::Arc::new(
            vw_platform::clipboard::ClipboardService::new()?,
        ));
    }
    Ok(config)
}

/// Executes one `secrets` request.
///
/// # Errors
/// [`ErrorCode::KeyService`] (no vault, locked, invalid name or value, not an own secret; reason word in
/// `server_code`), [`ErrorCode::NotFound`] for an unknown grant.
#[allow(
    clippy::too_many_lines,
    reason = "typed vault actions share one user-only dispatcher"
)]
pub(crate) async fn execute(
    shared: &Shared,
    request: SecretsRequest,
) -> Result<SecretsResult, CoreError> {
    let keys = || {
        shared
            .config
            .keys
            .as_ref()
            .ok_or_else(crate::keys::no_vault)
    };
    let map = |e: vw_keys::KeyError| crate::keys::key_error(&e);
    let result = match request {
        SecretsRequest::List {} => return list(shared, keys()?, None).await,
        SecretsRequest::ListForProject { project } => {
            return list(shared, keys()?, Some(&project)).await;
        }
        SecretsRequest::SetEntryPermission {
            project,
            id,
            permission,
        } => {
            keys()?
                .set_entry_permission(
                    crate::entry_permissions::user_project(shared, &project)?,
                    id,
                    permission.into(),
                )
                .await
                .map_err(map)?;
            SecretsResult::Done {}
        }
        SecretsRequest::SetDefaultPermission { id, permission } => {
            keys()?
                .set_default_entry_permission(id, permission.into())
                .await
                .map_err(map)?;
            SecretsResult::Done {}
        }
        SecretsRequest::ResetEntryPermission { project, id } => {
            keys()?
                .reset_entry_permission(
                    crate::entry_permissions::user_project(shared, &project)?,
                    id,
                )
                .await
                .map_err(map)?;
            SecretsResult::Done {}
        }
        SecretsRequest::SetAgentAccess { locked } => {
            crate::agent_access::set_for_user(shared, locked).await?;
            SecretsResult::Done {}
        }
        generated @ (SecretsRequest::Generate { .. }
        | SecretsRequest::GenerateWithPermission { .. }
        | SecretsRequest::Regenerate { .. }
        | SecretsRequest::GenerateLogin { .. }
        | SecretsRequest::GenerateLoginWithPermission { .. }
        | SecretsRequest::RegenerateLoginPassword { .. }) => {
            crate::secret_generate::execute(shared, keys()?, generated).await?
        }
        SecretsRequest::Add { name, value, label } => {
            let stored = keys()?
                .store_secret_labelled(
                    name,
                    value.into_secret(),
                    None,
                    label.as_deref().and_then(vw_keys::SecretLabel::parse),
                )
                .await
                .map_err(map)?;
            SecretsResult::Stored {
                secret: stored.secret.into(),
                replaced: stored.replaced,
            }
        }
        SecretsRequest::AddWithPermission {
            name,
            value,
            label,
            project,
            permission,
        } => {
            let project = crate::entry_permissions::user_project(shared, &project)?;
            let stored = keys()?
                .create_secret_labelled(
                    name,
                    value.into_secret(),
                    label.as_deref().and_then(vw_keys::SecretLabel::parse),
                )
                .await
                .map_err(map)?;
            if let Err(error) = keys()?
                .set_entry_permission(project.clone(), stored.secret.id.clone(), permission.into())
                .await
            {
                let _ = keys()?.remove_secret(stored.secret.id.clone()).await;
                return Err(map(error));
            }
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
        SecretsRequest::Remove { id } => {
            keys()?.remove_secret(id.clone()).await.map_err(map)?;
            let servers = crate::mcp_keys::revoke_secret(shared, &id).await;
            tracing::info!(servers, "project secret removed with its grants");
            SecretsResult::Done {}
        }
        SecretsRequest::Rename { id, name } => {
            keys()?.rename_secret(id, name).await.map_err(map)?;
            SecretsResult::Done {}
        }
        SecretsRequest::Replace { id, value } => {
            crate::secret_replace::confirm(shared, keys()?, &id, false).await?;
            keys()?
                .replace_secret(id, value.into_secret())
                .await
                .map_err(map)?;
            SecretsResult::Done {}
        }
        copy @ (SecretsRequest::Copy { .. }
        | SecretsRequest::CopyLoginUsername { .. }
        | SecretsRequest::CopyLoginPassword { .. }) => {
            return copy_to_clipboard(shared, keys()?, copy).await;
        }
        SecretsRequest::Revoke { grant_id } => {
            revoke_grants(shared, std::slice::from_ref(&grant_id)).await?;
            SecretsResult::Done {}
        }
        SecretsRequest::RevokeMany { grant_ids } => {
            if grant_ids.is_empty() || grant_ids.len() > 10_000 {
                return Err(CoreError::new(
                    ErrorCode::InvalidParams,
                    "Name between 1 and 10000 grants to revoke.",
                ));
            }
            revoke_grants(shared, &grant_ids).await?;
            SecretsResult::Done {}
        }
        login @ (SecretsRequest::AddLogin { .. }
        | SecretsRequest::AddLoginWithPermission { .. }
        | SecretsRequest::RemoveLogin { .. }
        | SecretsRequest::RenameLogin { .. }
        | SecretsRequest::ReplaceLoginPassword { .. }) => logins(shared, keys()?, login).await?,
    };
    shared.emit(CoreEvent::SecretsChanged {});
    Ok(result)
}

/// Revokes every grant in `ids` (MCP server key, website fill), refreshing Vibe's servers once when a key
/// grant went. Ids already gone are skipped.
///
/// # Errors
/// `notFound` when none of them was revoked; the vault's errors (locked, storage) stop at the first.
async fn revoke_grants(shared: &Shared, ids: &[String]) -> Result<(), CoreError> {
    let mut revoked = 0usize;
    let mut servers = false;
    for id in ids {
        // Only site and MCP grants remain; entry permission is edited directly.
        let done = if vw_vault::fill_grant::is_grant_id(id) {
            crate::fill_grants::revoke(shared, id).await?
        } else if vw_vault::mcp_binding::is_binding_id(id) {
            let done = crate::mcp_keys::revoke(shared, id).await?;
            servers |= done;
            done
        } else {
            false
        };
        revoked += usize::from(done);
    }
    if servers {
        crate::connectors::refresh_servers(shared).await;
    }
    if revoked == 0 {
        return Err(CoreError::new(ErrorCode::NotFound, "no such grant"));
    }
    Ok(())
}

/// Lists metadata and active grant views; a locked vault has no readable items or grants.
async fn list(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    project: Option<&str>,
) -> Result<SecretsResult, CoreError> {
    let project = project
        .map(|project| crate::entry_permissions::user_identity(shared, project))
        .transpose()?;
    let project = project.as_ref();
    let uses = shared.secret_uses.log_entries();
    let (unlocked, mut secrets): (bool, Vec<crate::secrets_view::ProjectSecretView>) =
        match keys.project_secrets().await {
            Ok(list) => (true, list.into_iter().map(Into::into).collect()),
            Err(e) if vw_keys::is_locked(&e) => (false, Vec::new()),
            Err(e) => return Err(crate::keys::key_error(&e)),
        };
    let (server_grants, mut logins, fill_grants): (_, Vec<crate::secrets_view::LoginView>, _) =
        if unlocked {
            (
                crate::mcp_keys::grant_views(shared).await,
                keys.logins()
                    .await
                    .map(|l| l.into_iter().map(Into::into).collect())
                    .unwrap_or_default(),
                crate::fill_grants::views(shared).await,
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
    if unlocked {
        let policies = keys
            .all_entry_permissions()
            .await
            .map_err(|e| crate::keys::key_error(&e))?;
        for secret in &mut secrets {
            let metadata = permission_metadata(keys, project, &secret.id, &policies).await?;
            (
                secret.permission,
                secret.default_permission,
                secret.project_permission,
                secret.project_permissions,
            ) = metadata;
        }
        for login in &mut logins {
            let metadata = permission_metadata(keys, project, &login.id, &policies).await?;
            (
                login.permission,
                login.default_permission,
                login.project_permission,
                login.project_permissions,
            ) = metadata;
        }
    }
    Ok(SecretsResult::List {
        unlocked,
        agent_access_locked: crate::agent_access::locked(shared),
        secrets,
        uses,
        server_grants,
        logins,
        fill_grants,
    })
}

/// Builds one row's default, selected project and explicit exception metadata without values.
async fn permission_metadata(
    keys: &vw_keys::KeyService,
    project: Option<&crate::entry_permissions::Identity>,
    id: &str,
    policies: &[vw_vault::entry_permission::EntryPolicy],
) -> Result<
    (
        Option<crate::secrets_view::EntryPermissionView>,
        crate::secrets_view::EntryPermissionView,
        Option<crate::secrets_view::EntryPermissionView>,
        Vec<crate::secrets_view::ProjectPermissionView>,
    ),
    CoreError,
> {
    use crate::secrets_view::{EntryPermissionView, ProjectPermissionView};
    let map = |e| crate::keys::key_error(&e);
    let default: EntryPermissionView = keys
        .default_entry_permission(id)
        .await
        .map_err(map)?
        .0
        .into();
    let own = if let Some(project) = project {
        keys.own_entry_permission(project.allowed_project(), id)
            .await
            .map_err(map)?
            .map(|(permission, _)| permission.into())
    } else {
        None
    };
    let effective = if let Some(project) = project {
        project
            .policy(keys, id)
            .await
            .map_err(map)?
            .permission
            .into()
    } else {
        default
    };
    let exceptions = policies
        .iter()
        .filter(|p| p.entry_id == id)
        .map(|p| ProjectPermissionView {
            project: p.project.clone(),
            permission: p.permission.into(),
        })
        .collect();
    Ok((Some(effective), default, own, exceptions))
}

/// Sends a user-selected listed value only to the native clipboard. A copy changes no vault state.
async fn copy_to_clipboard(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    request: SecretsRequest,
) -> Result<SecretsResult, CoreError> {
    let map = |e: vw_keys::KeyError| crate::keys::key_error(&e);
    let value = match request {
        SecretsRequest::Copy { id } => Zeroizing::new(
            keys.listed_secret_value(id)
                .await
                .map_err(map)?
                .expose()
                .to_owned(),
        ),
        SecretsRequest::CopyLoginUsername { id } => {
            Zeroizing::new(keys.listed_login_username(id).await.map_err(map)?)
        }
        SecretsRequest::CopyLoginPassword { id } => Zeroizing::new(
            keys.listed_login_password(id)
                .await
                .map_err(map)?
                .expose()
                .to_owned(),
        ),
        _ => {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "not a copy request",
            ));
        }
    };
    clipboard(shared, value).await?;
    Ok(SecretsResult::Done {})
}

/// Writes a generated or listed value through the native clipboard's conditional clear timer.
pub(crate) async fn clipboard(shared: &Shared, value: Zeroizing<String>) -> Result<(), CoreError> {
    shared
        .config
        .clipboard
        .as_ref()
        .ok_or_else(|| CoreError::new(ErrorCode::Internal, "clipboard is unavailable"))?
        .copy(value)
        .await
        .map_err(|_| CoreError::new(ErrorCode::Internal, "clipboard is unavailable"))
}

/// `addLogin` and `removeLogin` (a removed login's fill grants are revoked).
async fn logins(
    shared: &Shared,
    keys: &vw_keys::KeyService,
    request: SecretsRequest,
) -> Result<SecretsResult, CoreError> {
    let map = |e: vw_keys::KeyError| crate::keys::key_error(&e);
    Ok(match request {
        SecretsRequest::AddLogin {
            name,
            url,
            username,
            password,
        } => {
            let stored = keys
                .store_login(name, url, username, password.into_secret())
                .await
                .map_err(map)?;
            SecretsResult::LoginStored {
                login: stored.login.into(),
                replaced: stored.replaced,
            }
        }
        SecretsRequest::AddLoginWithPermission {
            name,
            url,
            username,
            password,
            project,
            permission,
        } => {
            let project = crate::entry_permissions::user_project(shared, &project)?;
            let stored = keys
                .create_login(name, url, username, password.into_secret())
                .await
                .map_err(map)?;
            if let Err(error) = keys
                .set_entry_permission(project.clone(), stored.login.id.clone(), permission.into())
                .await
            {
                let _ = keys.remove_login(stored.login.id.clone()).await;
                return Err(map(error));
            }
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
        SecretsRequest::RemoveLogin { id } => {
            keys.remove_login(id.clone()).await.map_err(map)?;
            let revoked = crate::fill_grants::revoke_login(shared, &id).await;
            tracing::info!(revoked, "login removed with its fill grants");
            SecretsResult::Done {}
        }
        SecretsRequest::RenameLogin { id, name } => {
            keys.rename_login(id, name).await.map_err(map)?;
            SecretsResult::Done {}
        }
        SecretsRequest::ReplaceLoginPassword { id, password } => {
            crate::secret_replace::confirm(shared, keys, &id, true).await?;
            keys.replace_login_password(id, password.into_secret())
                .await
                .map_err(map)?;
            SecretsResult::Done {}
        }
        _ => {
            return Err(CoreError::new(
                ErrorCode::InvalidParams,
                "not a login request",
            ));
        }
    })
}
