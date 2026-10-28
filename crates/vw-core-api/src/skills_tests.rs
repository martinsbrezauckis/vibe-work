//! The outcome of an offered skill update after the change ran: what Vibe reports installed decides the answer.
use serde_json::json;

fn board(installed: Option<i64>) -> vw_skills::SkillsView {
    let rows = installed.map_or_else(Vec::new, |version| {
        vec![json!({"name": "release-notes", "description": null, "source": null, "enabled": true,
            "installedVersion": version, "latestVersion": version, "updateAvailable": false,
            "installations": [], "active": null, "catalog": [], "updates": [], "catalogUpdate": null})]
    });
    serde_json::from_value(json!({"skills": rows, "issues": [], "catalogLoaded": true,
        "authenticated": true, "projectAvailable": false}))
    .unwrap()
}

#[test]
fn the_offered_version_installed_is_the_update_done() {
    assert!(super::offered_update_landed(&board(Some(5)), Some("release-notes"), 5).is_ok());
}

/// The update to v5 ran, but Vibe now reports v6 (another Vibe sharing the skills folder updated it in between) or
/// no copy at all. That is not the stale-offer refusal of a change that never ran ("Nothing was installed"): the
/// answer says the update ran and what Vibe reports now, and nothing is undone.
#[test]
fn another_version_after_the_update_is_told_as_what_happened() {
    for (installed, now) in [
        (Some(6), "Vibe now reports version 6 installed"),
        (None, "Vibe no longer reports this skill installed"),
    ] {
        let error = super::offered_update_landed(&board(installed), Some("release-notes"), 5)
            .expect_err("not the offered version");
        assert_ne!(
            error.server_code.as_deref(),
            Some(crate::install_offers::STALE_OFFER),
            "{error:?}"
        );
        assert_eq!(
            error.server_code.as_deref(),
            Some(super::UPDATED_OTHER_VERSION),
            "{error:?}"
        );
        assert!(
            error.message.contains("The update to version 5 ran") && error.message.contains(now),
            "{}",
            error.message
        );
    }
}

#[test]
fn skills_authentication_uses_the_apps_key_state_and_keeps_the_catalog() {
    for (vibe_authenticated, usable_key) in [(true, false), (false, true)] {
        let mut view = board(Some(5));
        view.authenticated = vibe_authenticated;
        let result = super::board(view, false, usable_key);
        assert_eq!(result.authenticated, usable_key);
        assert!(result.catalog_loaded);
        assert_eq!(result.skills[0].name, "release-notes");
    }
}
