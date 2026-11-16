//! Changing installed skills: which row an action targets, the install or update that follows the offered
//! or latest version, and the check that an offered update landed on the version the dialog named.
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::keys::mistral_key_available;
use crate::skills_rows::{board, lock_reason, representative, row_view};
use crate::skills_view::{SkillsAction, SkillsResult};
use std::path::Path;
use std::sync::Mutex;
use vw_skills::{SkillsClient, SkillsTransport};

use super::{BUSY, Transport, change_failure, refuse};

/// The registry change of an installed skill: remove it unless `updating`. An offered update (`pinned`) installs
/// and pins that version (`skills/setVersion`), because Vibe's `setLatest` fetches whatever `latest` names at that
/// moment and keeps following it; an update the native dialog confirmed follows `latest`.
async fn change_installed<T: SkillsTransport>(
    client: &SkillsClient<T>,
    id: &str,
    name: String,
    scope: String,
    updating: bool,
    pinned: Option<i64>,
) -> Result<(), vw_skills::SkillsError> {
    let session_id = id.to_owned();
    match (updating, pinned) {
        (true, Some(version)) => client
            .set_version(&vw_skills::SkillsSetVersionParams {
                session_id,
                name,
                version,
                scope,
            })
            .await
            .map(drop),
        (true, None) => client
            .set_latest(&vw_skills::SkillsSetLatestParams {
                session_id,
                name,
                scope,
            })
            .await
            .map(drop),
        (false, _) => client
            .remove(&vw_skills::SkillsRemoveParams {
                session_id,
                name,
                scope,
            })
            .await
            .map(drop),
    }
}

/// The `serverCode` of an offered update that ran while Vibe now reports another version (or none) installed.
pub(crate) const UPDATED_OTHER_VERSION: &str = "updatedOtherVersion";

/// An offered update counts as done only when Vibe now reports the offered version installed. Otherwise the change
/// did run (another Vibe sharing the skills folder can change it in between): the answer says so and what Vibe
/// reports now, and nothing is undone (a rollback could overwrite that other change). The window then reads the
/// list again.
fn offered_update_landed(
    view: &vw_skills::SkillsView,
    name: Option<&str>,
    version: i64,
) -> Result<(), CoreError> {
    let installed = view
        .skills
        .iter()
        .find(|r| Some(r.name.as_str()) == name)
        .and_then(|r| r.installed_version);
    if installed == Some(version) {
        return Ok(());
    }
    let now = installed.map_or_else(
        || "Vibe no longer reports this skill installed".to_owned(),
        |other| format!("Vibe now reports version {other} installed"),
    );
    let mut error = CoreError::new(
        ErrorCode::ServerError,
        format!(
            "The update to version {version} ran, but {now}. Nothing was undone; the list now shows what is installed."
        ),
    );
    error.server_code = Some(UPDATED_OTHER_VERSION.to_owned());
    Err(error)
}

pub(super) fn mutation_target<'a>(
    view: &'a vw_skills::SkillsView,
    action: &SkillsAction,
) -> Result<Option<&'a vw_skills::SkillView>, CoreError> {
    let name = match action {
        SkillsAction::SetEnabled { name, .. }
        | SkillsAction::SetLatest { name, .. }
        | SkillsAction::OfferUpdate { name }
        | SkillsAction::Remove { name } => Some(name.as_str()),
        SkillsAction::Import { skill_id, .. } | SkillsAction::OfferImport { skill_id } => view
            .skills
            .iter()
            .find(|r| r.catalog.iter().any(|c| &c.skill_id == skill_id))
            .map(|r| r.name.as_str()),
        _ => None,
    };
    let target = name.and_then(|n| view.skills.iter().find(|r| r.name == n));
    if !matches!(action, SkillsAction::Read {}) {
        let row = target
            .ok_or_else(|| refuse("The skill is no longer available. Read the list again."))?;
        if let Some(reason) = lock_reason(row) {
            return Err(refuse(reason));
        }
        if !row.installations.is_empty() && representative(row).is_none() {
            return Err(refuse(
                "This skill has multiple installed copies. Choose it in Vibe's settings file.",
            ));
        }
    }
    Ok(target)
}

impl Core {
    pub(super) async fn skills_in(
        &self,
        origin: Origin,
        id: &str,
        folder: &Path,
        action: SkillsAction,
    ) -> Result<SkillsResult, CoreError> {
        let before = self.skills_read(origin, id).await?;
        let refusal = Mutex::new(None);
        // an install or update naming a shown offer takes it first (single use, also when the change
        // fails); only the window's click then skips the native dialog.
        let (offered, pinned) = self.offered_skill_change(origin, &action, folder, &before)?;
        let client = SkillsClient::new(Transport {
            core: self,
            origin,
            offered,
            refusal: &refusal,
        });
        if let Some(answer) = self
            .skills_view_only(&client, id, folder, &before, &action)
            .await
        {
            return answer;
        }
        if !matches!(action, SkillsAction::Read {}) && self.skills_busy(folder) {
            return Err(refuse(BUSY));
        }
        let target = mutation_target(&before, &action)?;
        let row = target.map(row_view);
        let updating = matches!(action, SkillsAction::SetLatest { .. });
        let target_name = row.as_ref().map(|r| r.name.clone());
        let changed = match action {
            SkillsAction::Read {} => false,
            SkillsAction::SetEnabled { name, enabled } => {
                if !row.as_ref().is_some_and(|r| r.installed) {
                    return Err(refuse("Install this skill first."));
                }
                client
                    .set_enabled(&vw_skills::SkillsSetEnabledParams {
                        session_id: id.to_owned(),
                        name,
                        enabled,
                    })
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::Import { skill_id, .. } => {
                if row.as_ref().is_some_and(|r| r.installed) {
                    return Err(refuse("This skill is already installed."));
                }
                client
                    .import(&vw_skills::SkillsImportParams {
                        session_id: id.to_owned(),
                        skill_id,
                        // The version a shown offer named (the click installs what the user saw); latest
                        // otherwise (the native dialog path).
                        version: pinned,
                        alias: None,
                        scope: "global".to_owned(),
                    })
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::SetLatest { name, .. } | SkillsAction::Remove { name } => {
                let row = row.ok_or_else(|| refuse("The skill is unavailable."))?;
                if !row.installed || row.skill_id.is_none() {
                    return Err(refuse(
                        "Only registry-installed skills can be updated or removed.",
                    ));
                }
                let scope = row
                    .scope
                    .ok_or_else(|| refuse("The skill's install scope is ambiguous."))?;
                change_installed(&client, id, name, scope, updating, pinned)
                    .await
                    .map_err(|e| change_failure(&refusal, &e))?;
                true
            }
            SkillsAction::Detail { .. }
            | SkillsAction::OfferImport { .. }
            | SkillsAction::OfferUpdate { .. } => unreachable!(),
        };
        let view = if changed {
            self.skills_read(origin, id).await?
        } else {
            before
        };
        if let (true, Some(version)) = (updating, pinned) {
            offered_update_landed(&view, target_name.as_deref(), version)?;
        }
        let authenticated = mistral_key_available(self.shared.config.keys.as_deref());
        let busy = self.skills_busy(folder);
        let places =
            crate::skill_times::SkillPlaces::new(&self.shared.instructions.own_home, folder);
        // The rows' file times read the disk: off the async workers.
        let view = tokio::task::spawn_blocking(move || board(view, busy, authenticated, &places))
            .await
            .map_err(|e| CoreError::new(ErrorCode::Internal, format!("skills list failed: {e}")))?;
        Ok(SkillsResult::View { view })
    }
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
