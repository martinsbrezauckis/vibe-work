//! Skill install offers: the version an offer names, the offer card the native dialog shows (source, author,
//! what a skill adds), and spending a confirmed offer on exactly that version.
use crate::command::Origin;
use crate::core::Core;
use crate::error::CoreError;
use crate::skills_rows::row_view;
use crate::skills_view::SkillsAction;
use std::path::Path;
use vw_skills::{SkillsClient, SkillsTransport};

use super::{canonical, mutation_target, refuse};

/// The version an install offer names for `skill_id` of `row`: the catalog's latest, else the row's (one place, so
/// the offer and the check at the click read it the same way).
fn offered_version(
    row: &vw_skills::SkillView,
    skill_id: &str,
    row_latest: Option<i64>,
) -> Option<i64> {
    row.catalog
        .iter()
        .find(|c| c.skill_id == skill_id)
        .map(|c| c.latest_version)
        .or(row_latest)
}

impl Core {
    /// [`Self::take_skill_offer`] for an `import` or `setLatest` that names an offer: whether the click may skip the
    /// native dialog, and the version the change installs (the offered one). `(false, None)` for anything else.
    ///
    /// The offer is bound to what it showed: an import pins the offered version; an update is refused as a stale
    /// offer when the registry's latest is no longer the offered one in the view read right before (`before`), and
    /// otherwise installs the offered number with `skills/setVersion` (never the moving `latest`).
    pub(super) fn offered_skill_change(
        &self,
        origin: Origin,
        action: &SkillsAction,
        folder: &Path,
        before: &vw_skills::SkillsView,
    ) -> Result<(bool, Option<i64>), CoreError> {
        match action {
            SkillsAction::Import {
                skill_id,
                offer_id: Some(offer),
            } => self.take_skill_offer(origin, offer, "skills/import", skill_id, folder),
            SkillsAction::SetLatest {
                name,
                offer_id: Some(offer),
            } => {
                let (skip, shown) =
                    self.take_skill_offer(origin, offer, "skills/setLatest", name, folder)?;
                let now = mutation_target(before, action)?.and_then(|row| {
                    let view = row_view(row);
                    view.skill_id
                        .as_deref()
                        .and_then(|skill_id| offered_version(row, skill_id, view.latest_version))
                });
                if now != shown {
                    return Err(crate::install_offers::changed());
                }
                // The update pins the offered number (`skills/setVersion`); an offer without one is not installed.
                let version = shown.ok_or_else(crate::install_offers::changed)?;
                Ok((skip, Some(version)))
            }
            _ => Ok((false, None)),
        }
    }

    /// Takes the window's install offer for this exact action; true when the click may skip the native dialog
    /// (the window's origin only; the Debug API keeps its own confirmer path).
    fn take_skill_offer(
        &self,
        origin: Origin,
        offer: &str,
        method: &str,
        target: &str,
        folder: &Path,
    ) -> Result<(bool, Option<i64>), CoreError> {
        let key = crate::install_offers::OfferKey::new(
            method,
            target,
            &canonical(folder).to_string_lossy(),
        );
        let version =
            self.shared
                .install_offers
                .take(offer, &key, self.shared.asks.now_ms().into())?;
        Ok((origin == Origin::Ui, version))
    }

    /// The offer the window shows before Install or Update: Vibe's catalog name and version, the registry as the
    /// source, the author Vibe reports, and what a skill adds. Vibe's registry states no licence.
    pub(super) async fn skill_offer<T: SkillsTransport>(
        &self,
        client: &SkillsClient<T>,
        id: &str,
        folder: &Path,
        target: Option<&vw_skills::SkillView>,
        action: &SkillsAction,
    ) -> Result<crate::install_offers::ToolsInstallOffer, CoreError> {
        let row = target.ok_or_else(|| refuse("The skill is unavailable."))?;
        let shown = row_view(row);
        let (kind, method, key_target, skill_id) =
            if let SkillsAction::OfferImport { skill_id } = action {
                if shown.installed {
                    return Err(refuse("This skill is already installed."));
                }
                ("skill", "skills/import", skill_id.clone(), skill_id.clone())
            } else {
                if !shown.installed || shown.skill_id.is_none() || !shown.update_available {
                    return Err(refuse("There is no update for this skill."));
                }
                (
                    "skillUpdate",
                    "skills/setLatest",
                    row.name.clone(),
                    shown.skill_id.clone().unwrap_or_default(),
                )
            };
        let version = offered_version(row, &skill_id, shown.latest_version);
        let publisher = client
            .detail(&vw_skills::SkillsDetailParams {
                session_id: id.to_owned(),
                skill_id: skill_id.clone(),
                version,
            })
            .await
            .ok()
            .and_then(|d| d.detail)
            .map(|d| d.created_by)
            .filter(|by| !by.trim().is_empty());
        let offer = crate::install_offers::ToolsInstallOffer {
            offer_id: String::new(),
            kind: kind.to_owned(),
            name: row.name.clone(),
            publisher,
            source: format!("Mistral skill registry ({skill_id})"),
            licence: None,
            runs: "Instructions the agent may follow in every project. Installing runs no program."
                .to_owned(),
            version,
            expires_at: 0,
        };
        let key = crate::install_offers::OfferKey::new(
            method,
            &key_target,
            &canonical(folder).to_string_lossy(),
        );
        self.shared
            .install_offers
            .make(key, offer, self.shared.asks.now_ms().into())
    }
}
