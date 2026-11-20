//! The built-in extension floor: an extension older than 0.1.7 reads "Update the extension" even when the signed
//! release record sets no minimum, because the app and the helper no longer send the network rule such an extension
//! needed. PREDICT (before the change): 0.1.6 without a record minimum read "Connected" (no minimum, no update).
use super::*;

fn connected(installed: &str, min: Option<&str>) -> Facts {
    Facts {
        package: true,
        set_up: true,
        enabled: true,
        helper_up: true,
        extension_folder: true,
        connected: true,
        installed_version: Some(installed.into()),
        min_version: min.map(Into::into),
        ..Facts::default()
    }
}

fn unread() -> Probe {
    Probe::Unknown
}

#[test]
fn an_extension_below_the_floor_needs_an_update_without_a_record_minimum() {
    for old in ["0.1.6", "0.1.3", "0.0.9"] {
        assert_eq!(
            state(&connected(old, None), unread),
            AddonState::UpdateNeeded,
            "{old}"
        );
    }
}

#[test]
fn the_floor_and_the_record_minimum_both_apply() {
    for (installed, min, outdated) in [
        ("0.1.7", None, false),
        ("0.1.14", None, false),
        ("0.1.7", Some("0.1.4"), false),
        ("0.1.6", Some("0.1.4"), true),
        ("0.1.9", Some("0.1.12"), true),
        ("0.1.12", Some("0.1.12"), false),
    ] {
        let state = state(&connected(installed, min), unread);
        assert_eq!(
            state == AddonState::UpdateNeeded,
            outdated,
            "{installed} {min:?}: {state:?}"
        );
    }
}

#[test]
fn an_unreported_version_is_not_judged_by_the_floor() {
    let mut facts = connected("0.1.6", None);
    facts.installed_version = None;
    assert_ne!(state(&facts, unread), AddonState::UpdateNeeded);
}
