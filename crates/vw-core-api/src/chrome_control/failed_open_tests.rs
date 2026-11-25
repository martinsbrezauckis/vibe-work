//! Failed opens must not become durable selections through the next helper inventory.
use crate::chrome_control as control;
use crate::chrome_control::{
    owner,
    profile_binding_tests::{connected, open},
    profile_bindings as bindings, test_support,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[allow(
    clippy::unwrap_used,
    reason = "checked-in captured session fixture must be valid JSON"
)]
fn captured(profile: &str) -> Value {
    // Exact C3-025 details.session, with only the fixture's chat/profile identities substituted.
    let mut row: Value = serde_json::from_str(include_str!("failed_open_session.json")).unwrap();
    row["clientSessionId"] = json!("s");
    row["profileId"] = json!(profile);
    row
}
/// Agent-opened external browsers in this build: Firefox exists only with `your-firefox`.
fn external_profiles() -> Vec<&'static str> {
    let mut profiles = vec!["edge:work"];
    if cfg!(feature = "your-firefox") {
        profiles.insert(0, "firefox:agent");
    }
    profiles
}
fn status(rows: Value) -> Value {
    let mut reply = json!({"structuredContent":{"version":1,"clientId":"vibework-owner",
        "firefoxWindow":"Window open","profiles":[
            {"profileId":"chrome:work","connected":true},
            {"profileId":"edge:work","connected":true},
            {"profileId":"firefox:agent","connected":true}],"sessions":[]}});
    reply["structuredContent"]["sessions"] = rows;
    reply
}
fn not_ready(row: Value) -> Value {
    let mut reply = owner::failure(
        "The browser did not confirm the browser session",
        "SESSION_NOT_READY",
    );
    reply["structuredContent"]["error"]["details"]["session"] = row;
    reply
}

#[tokio::test]
async fn failed_open_after_unconfirmed_stop_does_not_rebind_or_persist() {
    for profile in external_profiles() {
        let core = connected(&format!("failed-open-terminal-{profile}")).await;
        let chrome = &core.shared.chrome_control;
        *chrome.owner.firefox_pairing.lock().unwrap() = (control::PairingState::WindowOpen, None);
        // Existing R54 unlock, then the failed reopen from B3.
        bindings::remember(chrome, "s", profile, Some("previous")).unwrap();
        let mut ended = captured(profile);
        ended["sessionId"] = json!("previous");
        ended["suspensionReason"] = json!("STOP_UNCONFIRMED");
        owner::update_work_for_test(&core.shared, &status(json!([ended])));
        assert!(!bindings::locked(chrome, "s"));
        let failed = Arc::new(AtomicBool::new(false));
        let sent = failed.clone();
        test_support::answering(&core.shared, move |tool, args| {
            if tool == "chrome_status" {
                status(if sent.load(Ordering::SeqCst) {
                    json!([captured(profile)])
                } else {
                    json!([])
                })
            } else if args["profileId"] == profile {
                sent.store(true, Ordering::SeqCst);
                not_ready(captured(profile))
            } else {
                json!({"structuredContent":{"sessionId":"new-live","profileId":args["profileId"],"state":"active"}})
            }
        });
        open(&core, profile).await.unwrap();
        assert!(!bindings::locked(chrome, "s"));
        assert_eq!(bindings::selected(chrome, "s"), None);
        assert!(
            !bindings::Bindings::load(chrome.dir())
                .chats()
                .contains_key("s")
        );
        // Resolve uses a fresh helper inventory before the next open, the original re-lock path.
        let next = open(&core, "chrome:work").await;
        assert!(next.is_ok(), "{profile}: {next:?}");
        assert_eq!(
            next.unwrap()["vwMcpResult"]["structuredContent"]["sessionId"],
            "new-live"
        );
        assert!(
            !control::profile_selection::view(chrome)
                .unwrap()
                .iter()
                .any(|row| row.session_id == "s"
                    && row.profile_id.as_deref() == Some(profile)
                    && row.state == "selected")
        );
        let bytes = std::fs::read(chrome.dir().unwrap().join("chat-profiles.json")).unwrap();
        let saved: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            saved["sessions"]
                .get(captured(profile)["sessionId"].as_str().unwrap())
                .is_none()
        );
        let reloaded = bindings::Bindings::load(chrome.dir());
        assert_eq!(
            reloaded.chats().get("s").map(String::as_str),
            Some("chrome:work")
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn failed_open_connecting_is_uncertain_until_a_terminal_inventory() {
    for profile in external_profiles() {
        let core = connected(&format!("failed-open-connecting-{profile}")).await;
        let chrome = &core.shared.chrome_control;
        *chrome.owner.firefox_pairing.lock().unwrap() = (control::PairingState::WindowOpen, None);
        test_support::answering(&core.shared, move |tool, _| {
            if tool == "chrome_status" {
                status(json!([]))
            } else {
                let mut row = captured(profile);
                row["state"] = json!("connecting");
                row["desiredState"] = json!("active");
                row["suspensionReason"] = json!("CONTROL_FENCE");
                not_ready(row)
            }
        });
        open(&core, profile).await.unwrap();
        assert!(bindings::uncertain(chrome, "s"));
        assert!(open(&core, "chrome:work").await.is_err());
        let mut wrong = captured(profile);
        wrong["clientSessionId"] = json!("another-chat");
        owner::update_work_for_test(&core.shared, &status(json!([wrong])));
        assert!(bindings::uncertain(chrome, "s"));
        let mut terminal = captured(profile);
        terminal["suspensionReason"] = json!("CONTROL_FENCE");
        owner::update_work_for_test(&core.shared, &status(json!([terminal])));
        assert!(!bindings::locked(chrome, "s"));
        assert!(bindings::check(chrome, "s", "chrome:work").is_ok());
        assert!(
            bindings::Bindings::load(chrome.dir())
                .check("s", "chrome:work")
                .is_ok()
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn uncertain_open_without_a_session_id_ends_on_complete_empty_inventory() {
    let core = connected("failed-open-no-receipt").await;
    let chrome = &core.shared.chrome_control;
    let old_status = tokio::time::Instant::now();
    bindings::mark_uncertain(chrome, "s", None, Some("edge:work"));
    owner::update_work(
        &Arc::downgrade(&core.shared),
        &status(json!([])),
        old_status,
    );
    assert!(
        bindings::locked(chrome, "s"),
        "a pre-open snapshot cannot end the hold"
    );
    let live = json!({"sessionId":"maybe","clientSessionId":"s","profileId":"edge:work","state":"connecting","desiredState":"active"});
    owner::update_work_for_test(&core.shared, &status(json!([live])));
    assert!(bindings::locked(chrome, "s"));
    // Missing chat attribution and partial action output are not proof of absence.
    owner::update_work_for_test(
        &core.shared,
        &status(json!([
            {"sessionId":"unattributed","profileId":"edge:work","state":"active"}
        ])),
    );
    assert!(bindings::locked(chrome, "s"));
    owner::update_work_for_test(&core.shared, &json!({"structuredContent":{"sessions":[]}}));
    assert!(bindings::locked(chrome, "s"));
    owner::update_work_for_test(&core.shared, &status(json!([])));
    assert!(!bindings::locked(chrome, "s"));
    core.shutdown().await;
}

#[tokio::test]
async fn failed_open_terminal_inventory_never_migrates_an_unconfirmed_session() {
    for state in ["disconnected", "stopped"] {
        let core = connected(&format!("failed-open-inventory-{state}")).await;
        test_support::answering(&core.shared, move |tool, args| {
            if tool == "chrome_status" {
                let mut row = captured("firefox:agent");
                row["state"] = json!(state);
                row["suspensionReason"] = json!("CONTROL_FENCE");
                status(json!([row]))
            } else {
                json!({"structuredContent":{"sessionId":"new-live","profileId":args["profileId"],"state":"active"}})
            }
        });
        let next = open(&core, "chrome:work").await;
        assert!(
            next.is_ok(),
            "an ended never-confirmed session must not seed a binding: {next:?}"
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn failed_open_preserves_a_prior_preference_and_no_session_is_not_a_hold() {
    let core = connected("failed-open-prior-preference").await;
    let chrome = &core.shared.chrome_control;
    bindings::remember(chrome, "s", "chrome:work", None).unwrap();
    test_support::answering(&core.shared, |tool, _| {
        if tool == "chrome_status" {
            status(json!([]))
        } else {
            not_ready(Value::Null)
        }
    });
    open(&core, "edge:work").await.unwrap();
    assert_eq!(
        bindings::selected(chrome, "s").as_deref(),
        Some("chrome:work")
    );
    assert!(!bindings::locked(chrome, "s"));
    assert_eq!(
        bindings::Bindings::load(chrome.dir())
            .chats()
            .get("s")
            .map(String::as_str),
        Some("chrome:work")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn uncertain_open_does_not_claim_a_prior_preference_is_live() {
    let core = connected("failed-open-uncertain-wording").await;
    let chrome = &core.shared.chrome_control;
    bindings::remember(chrome, "s", "chrome:work", None).unwrap();
    bindings::mark_uncertain(chrome, "s", Some("maybe"), Some("edge:work"));
    assert_eq!(
        bindings::check(chrome, "s", "chrome:personal").unwrap_err(),
        vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED
    );
    core.shutdown().await;
}

#[tokio::test]
async fn active_first_browser_still_refuses_a_second_with_vocabulary_text() {
    for profile in external_profiles() {
        let core = connected(&format!("failed-open-active-control-{profile}")).await;
        let chrome = &core.shared.chrome_control;
        *chrome.owner.firefox_pairing.lock().unwrap() = (control::PairingState::WindowOpen, None);
        open(&core, profile).await.unwrap();
        let refusal = open(&core, "chrome:work").await.unwrap_err();
        let expected = vw_vocabulary::texts::BROWSER_PROFILE_AGENT_REFUSAL
            .replace("{profile}", &control::profiles::label(chrome, profile));
        assert_eq!(
            refusal,
            format!(
                "{expected} To release this chat's hold, use chrome_session close_browser (profileId {profile}), then open a new browser session."
            )
        );
        core.shutdown().await;
    }
}
