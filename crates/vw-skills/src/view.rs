//! Plain manager projection of Vibe responses; the client never scans skill files.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::{
    ConfigIssue, SkillCatalogEntry, SkillScope, SkillSource, SkillSummary, SkillUpdateView,
    SkillsCatalogResponse, SkillsInstalledResponse, SkillsListResponse, SkillsUpdatesResponse,
};

/// Display source derived only from Vibe's source and scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillsSource {
    /// A shipped builtin.
    Builtin,
    /// An installed global skill, local or registry.
    User,
    /// An installed project skill, local or registry.
    Project,
    /// A skill supplied by a plugin.
    Plugin,
    /// A registry listing with no installed copy.
    Catalog,
}

/// One row per name, retaining Vibe's separate copies for scoped actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    /// Skill name supplied by Vibe.
    pub name: String,
    /// Active/unique copy's description; None if the replies are ambiguous.
    pub description: Option<String>,
    /// Display source of the active/unique copy; None if no winner is reported.
    pub source: Option<SkillsSource>,
    /// Enabled state of the active/unique copy; None for catalog-only/ambiguous rows.
    pub enabled: Option<bool>,
    /// Materialized version of the active/unique registry copy.
    pub installed_version: Option<i64>,
    /// Latest version for that registry identity, or a unique catalog-only entry.
    pub latest_version: Option<i64>,
    /// Vibe reported an update for this name, including a possibly shadowed pin.
    pub update_available: bool,
    /// Exact installed rows, including disabled and shadowed copies.
    pub installations: Vec<SkillSummary>,
    /// The authoritative skills/list entry, if supplied for this name.
    pub active: Option<SkillSummary>,
    /// Catalog entries sharing this name; identity is retained to avoid false joins.
    pub catalog: Vec<SkillCatalogEntry>,
    /// Detailed update signals as returned by Vibe.
    pub updates: Vec<SkillUpdateView>,
    /// Catalog's per-name update signal, without inventing a target scope.
    pub catalog_update: Option<i64>,
}

/// A serializable UI view; issues lack a skill id and remain session-level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsView {
    /// Rows ordered by name for stable rendering.
    pub skills: Vec<SkillView>,
    /// Exact runtime issues; no guessed attribution by file path.
    pub issues: Vec<ConfigIssue>,
    /// Vibe successfully loaded the catalog.
    pub catalog_loaded: bool,
    /// Vibe reports registry authentication.
    pub authenticated: bool,
    /// Vibe offers a distinct project install scope.
    pub project_available: bool,
}

impl SkillsView {
    /// Merge server replies, using skills/list for the active copy and retaining
    /// all installed scopes. With no reported winner, multiple copies stay ambiguous.
    #[must_use]
    pub fn merge(
        installed: &SkillsInstalledResponse,
        catalog: &SkillsCatalogResponse,
        updates: &SkillsUpdatesResponse,
        active: &SkillsListResponse,
        issues: &[ConfigIssue],
    ) -> Self {
        let names: BTreeSet<_> = installed
            .skills
            .iter()
            .map(|row| &row.name)
            .chain(active.skills.iter().map(|row| &row.name))
            .chain(catalog.skills.iter().map(|row| &row.name))
            .chain(updates.updates.iter().map(|row| &row.name))
            .chain(catalog.updates.keys())
            .collect();
        let skills = names
            .into_iter()
            .map(|name| {
                let installations: Vec<_> = installed
                    .skills
                    .iter()
                    .filter(|row| &row.name == name)
                    .cloned()
                    .collect();
                let current = unique(active.skills.iter().filter(|row| &row.name == name)).cloned();
                let entries: Vec<_> = catalog
                    .skills
                    .iter()
                    .filter(|row| &row.name == name)
                    .cloned()
                    .collect();
                let signals: Vec<_> = updates
                    .updates
                    .iter()
                    .filter(|row| &row.name == name)
                    .cloned()
                    .collect();
                let representative = current.as_ref().or_else(|| unique(installations.iter()));
                let registry = representative.and_then(|row| row.registry.as_ref());
                let latest_version = if let Some(registry) = registry {
                    unique(
                        entries
                            .iter()
                            .filter(|entry| entry.skill_id == registry.skill_id),
                    )
                    .map(|entry| entry.latest_version)
                    .or_else(|| {
                        // Updates carry no identity or scope; equal-version pins
                        // from different identities cannot be joined to one copy.
                        if installations
                            .iter()
                            .filter_map(|skill| skill.registry.as_ref())
                            .any(|pin| {
                                pin.version == registry.version && pin.skill_id != registry.skill_id
                            })
                        {
                            return None;
                        }
                        unique(
                            signals
                                .iter()
                                .filter(|update| update.current_version == registry.version),
                        )
                        .map(|update| update.latest_version)
                    })
                } else if representative.is_none() && installations.is_empty() {
                    unique(entries.iter()).map(|entry| entry.latest_version)
                } else {
                    None
                };
                let catalog_update = catalog.updates.get(name).copied();
                SkillView {
                    name: name.clone(),
                    description: representative
                        .map(|row| row.description.clone())
                        .or_else(|| {
                            if installations.is_empty() {
                                unique(entries.iter()).map(|entry| entry.description.clone())
                            } else {
                                None
                            }
                        }),
                    source: representative.map(display_source).or({
                        if installations.is_empty() && !entries.is_empty() {
                            Some(SkillsSource::Catalog)
                        } else {
                            None
                        }
                    }),
                    enabled: representative.map(|row| row.enabled),
                    installed_version: registry.map(|reference| reference.version),
                    latest_version,
                    update_available: catalog_update.is_some() || !signals.is_empty(),
                    installations,
                    active: current,
                    catalog: entries,
                    updates: signals,
                    catalog_update,
                }
            })
            .collect();
        Self {
            skills,
            issues: issues.to_vec(),
            catalog_loaded: catalog.loaded,
            authenticated: catalog.authenticated,
            project_available: catalog.project_available,
        }
    }
}

fn unique<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
    let first = items.next()?;
    if items.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn display_source(skill: &SkillSummary) -> SkillsSource {
    match (skill.source, skill.scope) {
        (SkillSource::Plugin, _) => SkillsSource::Plugin,
        (SkillSource::Builtin, _) | (_, SkillScope::Builtin) => SkillsSource::Builtin,
        (_, SkillScope::Project) => SkillsSource::Project,
        (_, SkillScope::Global) => SkillsSource::User,
    }
}
