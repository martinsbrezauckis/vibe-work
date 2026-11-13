//! Profile Settings contracts through the public command boundary and authenticated helper observations.
use super::{edge_agent_tests::fixture, test_support};
use serde_json::{Value, json};

#[allow(clippy::unwrap_used, reason = "test command input must decode")]
async fn command(
    core: &crate::Core,
    request: Value,
) -> Result<crate::CommandResult, crate::CoreError> {
    let command =
        serde_json::from_value(json!({"type":"chromeControl","request":request})).unwrap();
    core.execute(crate::Origin::Ui, command).await
}

#[tokio::test]
async fn settings_keep_named_profiles_separate_with_persisted_names_and_defaults() {
    let core = fixture(
        "profile-settings-rows",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Work","connected":true},
            {"profileId":"edge:work","name":"Work","connected":true}
        ]),
    )
    .await;
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    let rows = view["pairedProfiles"]
        .as_array()
        .expect("Settings must show each paired identity");
    assert_eq!(rows.len(), 3);
    assert_ne!(
        rows[0]["name"], rows[1]["name"],
        "duplicate Chrome labels need stable suffixes"
    );
    assert_eq!(
        rows.iter().filter(|row| row["browser"] == "chrome").count(),
        2
    );
    // Names come from the browser only: there is no rename command any more.
    assert!(
        serde_json::from_value::<crate::Command>(json!({"type":"chromeControl","request":
            {"op":"renameProfile","profileId":"chrome:work","name":"Business"}}))
        .is_err(),
        "a rename request does not decode"
    );
    command(
        &core,
        json!({"op":"setDefaultProfile","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    let work = view["pairedProfiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["profileId"] == "chrome:work")
        .unwrap();
    let before = rows
        .iter()
        .find(|row| row["profileId"] == "chrome:work")
        .unwrap();
    assert_eq!(work["name"], before["name"]);
    assert_eq!(work["isDefault"], true);
    let loaded = super::profiles::Profiles::load(core.shared.chrome_control.dir());
    assert_eq!(json!(loaded.name("chrome:work")), work["name"]);
    assert_eq!(
        loaded.defaults.get("chrome").map(String::as_str),
        Some("chrome:work")
    );
    test_support::observe_status(&core.shared, &json!({"profiles":[]}));
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    assert_eq!(
        view["pairedProfiles"].as_array().unwrap().len(),
        3,
        "offline identities stay visible"
    );
    core.shutdown().await;
}

/// "Ask in each chat" in Settings clears this browser's default; the other browser's default stays.
///
/// PREDICT (written before the change): fails. The request has no `clearDefaultProfile` op, so the command does not
/// decode (`unwrap` on `from_value` panics) and once a default is set nothing returns the browser to asking.
#[tokio::test]
async fn ask_in_each_chat_clears_only_this_browsers_default() {
    let core = fixture(
        "profile-settings-clear-default",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true},
            {"profileId":"edge:work","name":"Work","connected":true}
        ]),
    )
    .await;
    for id in ["chrome:work", "edge:work"] {
        command(&core, json!({"op":"setDefaultProfile","profileId":id}))
            .await
            .unwrap();
    }
    command(
        &core,
        json!({"op":"clearDefaultProfile","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    let default_of = |id: &str| {
        view["pairedProfiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["profileId"] == id)
            .unwrap()["isDefault"]
            .clone()
    };
    assert_eq!(default_of("chrome:work"), false);
    assert_eq!(default_of("edge:work"), true);
    let loaded = super::profiles::Profiles::load(core.shared.chrome_control.dir());
    assert_eq!(loaded.defaults.get("chrome"), None);
    assert_eq!(
        loaded.defaults.get("edge").map(String::as_str),
        Some("edge:work")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn turning_one_profile_off_does_not_disable_its_sibling() {
    let core = fixture(
        "profile-settings-off",
        json!([
            {"profileId":"chrome:work","name":"Work","connected":true},
            {"profileId":"chrome:personal","name":"Personal","connected":true}
        ]),
    )
    .await;
    test_support::answering(&core.shared, |tool, _| {
        if tool == "owner.browserAvailability" {
            json!({"applied":true})
        } else {
            json!({"structuredContent":{}})
        }
    });
    command(
        &core,
        json!({"op":"setProfileEnabled","profileId":"chrome:work","enabled":false}),
    )
    .await
    .unwrap();
    let view = serde_json::to_value(super::commands::view(&core.shared)).unwrap();
    let rows = view["pairedProfiles"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row["profileId"] == "chrome:work")
            .unwrap()["enabled"],
        false
    );
    assert_eq!(
        rows.iter()
            .find(|row| row["profileId"] == "chrome:personal")
            .unwrap()["eligible"],
        true
    );
    assert!(core.shared.chrome_control.enabled());
    core.shutdown().await;
}
