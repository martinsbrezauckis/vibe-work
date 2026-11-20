//! Which profiles the profile card offers: an offline profile is never a choice, a single paired profile needs no
//! card, and a default whose window is closed is never replaced silently.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use super::profile_choice_tests::{answer, card, core, core_with, open, selected};
use serde_json::json;
use std::time::Duration;

/// An offline profile is never a choice: the card offers only profiles that are usable now. A chat whose profile the
/// browser deleted gets no card when no other profile is connected.
///
/// PREDICT (written before the change): fails. With no default, one connected and one offline paired profile, the
/// base binds the chat to the connected one without a card (`choice_needed` counts connected profiles only), so the
/// person never learns that the other paired profile was skipped. After the change that open asks on the card, which
/// offers only the connected profile and says the other profiles' windows are closed.
#[tokio::test]
async fn an_offline_profile_is_not_offered_but_the_card_still_asks() {
    let (core, _) = core_with(
        "profile-choice-asleep",
        json!({"profiles":[{"profileId":"chrome:work","connected":true},{"profileId":"chrome:personal","connected":false}]}),
    )
    .await;
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    let effect = &entry["detail"]["effect"];
    let offered: Vec<_> = effect["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["profileId"].clone())
        .collect();
    assert_eq!(offered, [json!("chrome:work")], "{entry}");
    assert_eq!(effect["closedOthers"], true, "{entry}");
    assert!(effect["closedDefault"].is_null(), "{entry}");
    answer(
        &core.shared,
        entry["callbackId"].as_str().unwrap(),
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:work"}),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:work"));
    // A chat bound to a profile the browser deleted: the only other profile is offline, so there is nothing to offer.
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut("chrome:work")
        .unwrap()
        .deleted = true;
    assert!(
        super::profile_seeding::choice_needed(&core.shared, "s", &json!({"profileId":"chrome"}))
            .is_none()
    );
    core.shutdown().await;
}

/// With no default and exactly one paired profile there is nothing to choose: no card, the open runs there.
#[tokio::test]
async fn one_paired_profile_needs_no_card() {
    let (core, seen) = core("profile-choice-single").await;
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut("chrome:personal")
        .unwrap()
        .removed = true;
    assert!(
        super::profile_seeding::choice_needed(&core.shared, "s", &json!({"profileId":"chrome"}))
            .is_none(),
        "one paired profile needs no card"
    );
    tokio::time::timeout(Duration::from_secs(10), open(&core.shared))
        .await
        .expect("no card holds the open")
        .unwrap()
        .unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "open" && args["profileId"] == "chrome:work")
    );
    core.shutdown().await;
}

/// An explicit default is never silently replaced: when its window is closed the card asks, offering the connected
/// profiles and saying the default's window is closed; the chat binds only to what the person picks.
///
/// PREDICT (written before the change): fails. With Default = Work offline and Personal connected, `choice_needed`
/// sees one connected profile and asks nothing, and `resolve` takes the only eligible row, so the open runs in
/// Personal without any card (`card()` panics "no browser profile card opened"); `resolve` called directly binds
/// Personal instead of refusing.
#[tokio::test]
async fn a_default_whose_window_is_closed_is_never_replaced_silently() {
    let helper = json!({"profiles":[
        {"profileId":"chrome:work","connected":false},
        {"profileId":"chrome:personal","connected":true}
    ]});
    let (core, seen) = core_with("profile-choice-closed-default", helper.clone()).await;
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .defaults
        .insert("chrome".into(), "chrome:work".into());
    // The binding step itself never picks past the default, whichever route reaches it.
    let mut args = json!({"action":"open","profileId":"chrome"});
    let refused =
        super::profile_seeding::resolve(&core.shared, "s", &mut args, &helper).unwrap_err();
    assert!(
        refused.contains("Work") && refused.contains("no action ran"),
        "{refused}"
    );
    assert_eq!(selected(&core), None);
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    let effect = &entry["detail"]["effect"];
    let offered: Vec<_> = effect["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["profileId"].clone())
        .collect();
    assert_eq!(offered, [json!("chrome:personal")], "{entry}");
    assert_eq!(effect["closedDefault"], "Work", "{entry}");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "open"),
        "nothing opens before the person answers"
    );
    answer(
        &core.shared,
        entry["callbackId"].as_str().unwrap(),
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:personal"}),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    // The default stays the person's default for later chats.
    assert_eq!(
        core.shared
            .chrome_control
            .profiles
            .lock()
            .unwrap()
            .defaults
            .get("chrome")
            .map(String::as_str),
        Some("chrome:work")
    );
    core.shutdown().await;
}
