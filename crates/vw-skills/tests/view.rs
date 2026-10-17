//! Exercises the manager projection using tagged-model fixtures. Public replies
//! are the sole input: no directory or configuration discovery occurs here.

#![allow(clippy::unwrap_used)]

use serde_json::Value;
use vw_skills::{
    RuntimeMutationResponse, SkillScope, SkillUpdateView, SkillsCatalogResponse,
    SkillsInstalledResponse, SkillsListResponse, SkillsSource, SkillsUpdatesResponse, SkillsView,
};

fn result<T: serde::de::DeserializeOwned>(json: &str) -> T {
    let fixture: Value = serde_json::from_str(json).unwrap();
    serde_json::from_value(fixture["result"].clone()).unwrap()
}

fn inputs() -> (
    SkillsInstalledResponse,
    SkillsCatalogResponse,
    SkillsUpdatesResponse,
    SkillsListResponse,
) {
    (
        result(include_str!("fixtures/installed.json")),
        result(include_str!("fixtures/catalog.json")),
        result(include_str!("fixtures/updates.json")),
        result(include_str!("fixtures/list.json")),
    )
}

#[test]
fn merges_update_disabled_project_shadow_and_reported_sources() {
    let (installed, catalog, updates, active) = inputs();
    let mutation: RuntimeMutationResponse = result(include_str!("fixtures/import.json"));
    let view = SkillsView::merge(
        &installed,
        &catalog,
        &updates,
        &active,
        &mutation.runtime.issues,
    );
    assert_eq!(view.skills.len(), 5);
    let review = view.skills.iter().find(|row| row.name == "review").unwrap();
    assert_eq!(review.description.as_deref(), Some("Project review"));
    assert_eq!(review.source, Some(SkillsSource::Project));
    assert_eq!(review.installed_version, Some(3));
    assert_eq!(review.latest_version, Some(4));
    assert!(review.update_available);
    assert_eq!(review.installations.len(), 2);
    assert_eq!(review.installations[0].scope, SkillScope::Global);
    assert_eq!(review.installations[1].scope, SkillScope::Project);
    assert_eq!(
        review.installations[0].registry.as_ref().unwrap().version,
        2
    );
    let draft = view.skills.iter().find(|row| row.name == "draft").unwrap();
    assert_eq!(draft.enabled, Some(false));
    assert_eq!(draft.source, Some(SkillsSource::User));
    assert_eq!(draft.installed_version, None);
    let available = view
        .skills
        .iter()
        .find(|row| row.name == "catalog-only")
        .unwrap();
    assert_eq!(available.source, Some(SkillsSource::Catalog));
    assert_eq!(available.enabled, None);
    assert_eq!(available.latest_version, Some(1));
    assert_eq!(available.installations.len(), 0);
    assert!(!available.update_available);
    assert_eq!(
        view.skills
            .iter()
            .find(|row| row.name == "vibe")
            .unwrap()
            .source,
        Some(SkillsSource::Builtin)
    );
    assert_eq!(
        view.skills
            .iter()
            .find(|row| row.name == "plugin:notes")
            .unwrap()
            .source,
        Some(SkillsSource::Plugin)
    );
    assert_eq!(view.issues, mutation.runtime.issues);
    assert!(view.catalog_loaded && view.authenticated && view.project_available);
}

#[test]
fn empty_and_unavailable_catalog_are_distinct() {
    let mut catalog: SkillsCatalogResponse =
        serde_json::from_value(serde_json::json!({"skills": []})).unwrap();
    let installed = SkillsInstalledResponse { skills: vec![] };
    let active = SkillsListResponse { skills: vec![] };
    let updates = SkillsUpdatesResponse { updates: vec![] };
    let unavailable = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    assert_eq!(unavailable.skills.len(), 0);
    assert!(!unavailable.catalog_loaded);
    catalog.loaded = true;
    catalog.authenticated = false;
    let unauthenticated = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    assert!(unauthenticated.catalog_loaded);
    assert!(!unauthenticated.authenticated);
}

#[test]
fn uses_reported_active_copy_even_when_installed_order_changes() {
    let (mut installed, catalog, updates, active) = inputs();
    installed.skills.reverse();
    let view = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    assert_eq!(
        view.skills
            .iter()
            .find(|row| row.name == "review")
            .unwrap()
            .source,
        Some(SkillsSource::Project)
    );
}

#[test]
fn ambiguous_copies_and_registry_id_collisions_do_not_get_guessed_values() {
    let (installed, mut catalog, updates, mut active) = inputs();
    active.skills.clear();
    let view = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    let review = view.skills.iter().find(|row| row.name == "review").unwrap();
    assert_eq!(review.source, None);
    assert_eq!(review.enabled, None);
    assert_eq!(review.installed_version, None);
    assert_eq!(review.latest_version, None);
    assert_eq!(review.installations.len(), 2);
    let (_, _, _, active) = inputs();
    catalog.skills[0].skill_id = "different-id".to_owned();
    let view = SkillsView::merge(
        &installed,
        &catalog,
        &SkillsUpdatesResponse { updates: vec![] },
        &active,
        &[],
    );
    assert_eq!(
        view.skills
            .iter()
            .find(|row| row.name == "review")
            .unwrap()
            .latest_version,
        None
    );
}

#[test]
fn update_without_identity_does_not_choose_between_equal_version_pins() {
    let (mut installed, mut catalog, updates, active) = inputs();
    installed.skills[0].registry.as_mut().unwrap().version = 3;
    catalog.skills.clear();
    let view = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    let review = view.skills.iter().find(|row| row.name == "review").unwrap();
    assert_eq!(review.latest_version, None);
    assert!(review.update_available);
    assert_eq!(review.updates, updates.updates);
}

#[test]
fn latest_version_alone_is_not_an_update_signal_and_stale_signals_stay_visible() {
    let (installed, mut catalog, _, active) = inputs();
    catalog.updates.clear();
    let view = SkillsView::merge(
        &installed,
        &catalog,
        &SkillsUpdatesResponse { updates: vec![] },
        &active,
        &[],
    );
    assert!(
        !view
            .skills
            .iter()
            .find(|row| row.name == "review")
            .unwrap()
            .update_available
    );
    let updates = SkillsUpdatesResponse {
        updates: vec![SkillUpdateView {
            name: "orphan".into(),
            current_version: 1,
            latest_version: 2,
        }],
    };
    let view = SkillsView::merge(&installed, &catalog, &updates, &active, &[]);
    let orphan = view.skills.iter().find(|row| row.name == "orphan").unwrap();
    assert!(orphan.update_available);
    assert_eq!(orphan.installations.len(), 0);
    assert_eq!(orphan.source, None);
    assert_eq!(orphan.installed_version, None);
    assert_eq!(orphan.latest_version, None);
}
