//! The display projection of vw-skills rows for the Tools Skills tab: the row a copy resolves to, its lock, and
//! the list view with each row's last-modified time.
//! Depends on: vw-skills (Vibe's answers), `skills_view` (wire types), `skill_times` (file times). Called by:
//! `skills`.
//! Invariant: identities, scopes and versions come only from Vibe; the time is the only fact read from disk.
use crate::skill_times::SkillPlaces;
use crate::skills_view::{SkillIssue, SkillRow, SkillsView};

/// The active copy of a row, else its only installed copy.
pub(crate) fn representative(row: &vw_skills::SkillView) -> Option<&vw_skills::SkillSummary> {
    row.active
        .as_ref()
        .or_else(|| (row.installations.len() == 1).then(|| &row.installations[0]))
}
/// Why a row cannot be changed here: a plugin or Vibe's settings file sets it.
pub(crate) fn lock_reason(row: &vw_skills::SkillView) -> Option<String> {
    if let Some(skill) = representative(row) {
        match skill.scope {
            vw_skills::SkillScope::Session => return Some("Managed by this chat".to_owned()),
            vw_skills::SkillScope::Remote => return Some("Managed remotely".to_owned()),
            _ => {}
        }
    }
    row.installations
        .iter()
        .chain(row.active.iter())
        .find(|s| s.locked)
        .map(|s| {
            if s.source == vw_skills::SkillSource::Plugin {
                format!(
                    "Set by plugin {}",
                    s.name.split(':').next().unwrap_or(&s.name)
                )
            } else {
                "Set in Vibe's settings file".to_owned()
            }
        })
}

/// A row's projection without the file time (targets and details need none).
pub(crate) fn row_view(row: &vw_skills::SkillView) -> SkillRow {
    let current = representative(row);
    let installed = !row.installations.is_empty() || row.active.is_some();
    let lock_reason = lock_reason(row);
    let scope = current.map(|s| {
        match s.scope {
            vw_skills::SkillScope::Builtin => "builtin",
            vw_skills::SkillScope::Global => "global",
            vw_skills::SkillScope::Project => "project",
            vw_skills::SkillScope::Session => "session",
            vw_skills::SkillScope::Remote => "remote",
        }
        .to_owned()
    });
    let skill_id = current
        .and_then(|s| s.registry.as_ref().map(|r| r.skill_id.clone()))
        .or_else(|| {
            (!installed && row.catalog.len() == 1).then(|| row.catalog[0].skill_id.clone())
        });
    SkillRow {
        name: row.name.clone(),
        description: row.description.clone().unwrap_or_default(),
        source: if current.is_some_and(|s| s.scope == vw_skills::SkillScope::Builtin) {
            Some("builtin".to_owned())
        } else {
            row.source.map(|s| {
                match s {
                    vw_skills::SkillsSource::Builtin => "builtin",
                    vw_skills::SkillsSource::User => "user",
                    vw_skills::SkillsSource::Project => "project",
                    vw_skills::SkillsSource::Session => "session",
                    vw_skills::SkillsSource::Plugin => "plugin",
                    vw_skills::SkillsSource::Catalog => "catalog",
                }
                .to_owned()
            })
        },
        installed,
        enabled: row.enabled,
        locked: lock_reason.is_some() || (installed && current.is_none()),
        lock_reason,
        scope,
        skill_id,
        installed_version: row.installed_version,
        latest_version: row.latest_version,
        update_available: row.catalog_update.is_some()
            && row.catalog_update == row.latest_version
            && row
                .latest_version
                .zip(row.installed_version)
                .is_some_and(|(latest, installed)| latest > installed)
            && current.is_some_and(|s| s.registry.is_some()),
        modified_at: None,
    }
}

/// The Skills tab's list: every row with its skill file's last-modified time (`places`).
pub(crate) fn board(
    view: vw_skills::SkillsView,
    busy: bool,
    authenticated: bool,
    places: &SkillPlaces,
) -> SkillsView {
    SkillsView {
        skills: view
            .skills
            .iter()
            .map(|row| SkillRow {
                modified_at: representative(row).and_then(|copy| places.modified_ms(copy)),
                ..row_view(row)
            })
            .collect(),
        issues: view
            .issues
            .into_iter()
            .map(|i| SkillIssue {
                file: i.file,
                message: i.message,
            })
            .collect(),
        catalog_loaded: view.catalog_loaded,
        authenticated,
        busy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_skills_keep_their_scope_and_have_no_global_actions_or_file_time() {
        for source in ["local", "registry"] {
            let skill: vw_skills::SkillSummary = serde_json::from_value(json!({
                "name":"chat-skill", "description":"Current chat", "prompt":"Do work",
                "source":source, "scope":"session",
                "registry": if source == "registry" { json!({"skillId":"skill-chat", "version":1}) } else { json!(null) }
            })).unwrap();
            let skills = vec![skill.clone()];
            let view = vw_skills::SkillsView::merge(
                &vw_skills::SkillsInstalledResponse {
                    skills: skills.clone(),
                    issues: vec![],
                },
                &serde_json::from_value(json!({"skills":[]})).unwrap(),
                &vw_skills::SkillsUpdatesResponse { updates: vec![] },
                &vw_skills::SkillsListResponse { skills },
                &[],
            );
            let row = row_view(&view.skills[0]);
            assert_eq!(row.source.as_deref(), Some("session"));
            assert_eq!(row.scope.as_deref(), Some("session"));
            assert!(row.locked);
            assert_eq!(row.lock_reason.as_deref(), Some("Managed by this chat"));
            let unused = std::path::Path::new("unused-session-skill-location");
            assert_eq!(SkillPlaces::new(unused, unused).modified_ms(&skill), None);
        }
    }

    #[test]
    fn remote_skills_show_remote_scope_and_refuse_local_changes_or_file_time() {
        for source in ["local", "registry", "plugin", "builtin"] {
            let skill: vw_skills::SkillSummary = serde_json::from_value(json!({
                "name":"remote-skill", "description":"Remote skill", "prompt":"Do work",
                "source": source, "scope":"remote",
                "registry": if source == "registry" { json!({"skillId":"remote-id", "version":1}) } else { json!(null) }
            }))
            .unwrap();
            let skills = vec![skill.clone()];
            let view = vw_skills::SkillsView::merge(
                &vw_skills::SkillsInstalledResponse {
                    skills: skills.clone(),
                    issues: vec![],
                },
                &serde_json::from_value(json!({"skills":[]})).unwrap(),
                &vw_skills::SkillsUpdatesResponse { updates: vec![] },
                &vw_skills::SkillsListResponse { skills },
                &[],
            );
            let row = row_view(&view.skills[0]);
            assert_eq!(row.source, None);
            assert_eq!(row.scope.as_deref(), Some("remote"));
            assert!(row.locked);
            assert_eq!(row.lock_reason.as_deref(), Some("Managed remotely"));
            let home = tempfile::tempdir().unwrap();
            let local = home.path().join("skills/remote-skill/SKILL.md");
            let cached = home
                .path()
                .join("skills-registry-cache/store/remote-id/1/SKILL.md");
            for file in [&local, &cached] {
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, "---\nname: remote-skill\n---\nRemote skill").unwrap();
            }
            assert_eq!(
                SkillPlaces::new(home.path(), home.path()).modified_ms(&skill),
                None
            );
        }
    }

    #[test]
    fn active_only_remote_catalog_collision_stays_unavailable_and_locked() {
        let remote: vw_skills::SkillSummary = serde_json::from_value(json!({
            "name":"same-name", "description":"Remote skill", "prompt":"Do work",
            "source":"registry", "scope":"remote"
        }))
        .unwrap();
        let catalog = serde_json::from_value(json!({
            "skills":[{"name":"same-name", "skillId":"catalog-id", "description":"Catalog skill", "latestVersion":1, "sharingScope":""}]
        }))
        .unwrap();
        let view = vw_skills::SkillsView::merge(
            &vw_skills::SkillsInstalledResponse {
                skills: vec![],
                issues: vec![],
            },
            &catalog,
            &vw_skills::SkillsUpdatesResponse { updates: vec![] },
            &vw_skills::SkillsListResponse {
                skills: vec![remote],
            },
            &[],
        );
        let row = row_view(&view.skills[0]);
        assert_eq!(row.source, None);
        assert_eq!(row.scope.as_deref(), Some("remote"));
        assert!(row.locked);
        assert_eq!(row.lock_reason.as_deref(), Some("Managed remotely"));
    }
}
