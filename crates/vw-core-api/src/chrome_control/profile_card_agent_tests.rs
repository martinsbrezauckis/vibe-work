//! What the agent is told around the browser profile card, and the profile names it uses.
//!
//! PREDICT (written before the change): the first four tests fail on the base.
//! - A No on the profile card returns the generic decline ("Explain why it is needed for the user's delegated task
//!   ... The user can choose a default profile in Settings > Browser"), which reads as an invitation to retry, so the
//!   agent opened the card again in the same turn. The decline test fails on the text.
//! - A second open in the same turn after that No opens a second card: the refusal test times out waiting for an
//!   error and finds a second open card.
//! - After the person picked another profile than the agent's call named, that call is refused with "This chat uses
//!   another browser profile ...", which names no profile; the agent then said the other profile "belongs to another
//!   chat". The picked-profile test fails on the text.
//! - Status `profileSelection` says only that a card asks; it does not tell the agent to open with the browser's name
//!   so the person picks (the agent kept naming one profile's exact ID).
//! - Fallback "Chrome profile N" numbers count only the profiles shown, so a profile named "Chrome profile 3" became
//!   "Chrome profile 2" once another profile was removed, and a re-paired folder (a new identity in the same profile
//!   folder) took a new position. The stable-number test fails on "Chrome profile 2".
//!
//! The email-named test is a guard (green on the base since agent names follow the browser's own name): a profile the
//! person named with an email keeps that name in agent text, never a numbered alias.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use super::test_support;
use crate::ask::tool_calls;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use vw_ask::AnsweredBy;

const DECLINED: &str = "The user chose not to use Chrome in this chat now. Do not open it again or ask about browser profiles unless the user asks.";

fn status() -> Value {
    json!({"profiles":[
        {"profileId":"chrome:work","connected":true},
        {"profileId":"chrome:personal","connected":true}
    ]})
}

/// Chat `s` with `turn` as its running turn.
fn turn(shared: &crate::core::Shared, turn: &str, kind: vw_session::LoadKind) {
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s","status":{"activeTurnId":turn}},"history":[],"turns":[]}),
            kind,
        )
        .unwrap();
}

async fn core(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
    })
    .await;
    test_support::chat(&core.shared, "s");
    turn(&core.shared, "turn-1", vw_session::LoadKind::Replace);
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return json!({"structuredContent":status()});
        }
        if args["action"] == "open" {
            let task = format!("task-{}", args["profileId"].as_str().unwrap_or_default());
            return json!({"structuredContent":{"sessionId":task,"profileId":args["profileId"],"state":"active"}});
        }
        json!({"structuredContent":{}})
    });
    test_support::observe_status(&core.shared, &status());
    {
        let mut saved = core.shared.chrome_control.profiles.lock().unwrap();
        for (id, name, account) in [
            ("chrome:work", "Work", Some("work@example.com")),
            ("chrome:personal", "Personal", None),
        ] {
            let record = saved.records.get_mut(id).unwrap();
            record.browser_name = Some(name.into());
            record.account = account.map(str::to_owned);
        }
    }
    core
}

/// The browser profile cards of chat `s` so far: (callback id, status).
fn cards(shared: &Arc<crate::core::Shared>) -> Vec<(String, Value)> {
    let state = crate::sessions::snapshot(shared, "s").unwrap();
    serde_json::to_value(&state)
        .unwrap()
        .pointer("/state/history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry["detail"]["effect"]["kind"] == "browserProfile")
        .map(|entry| {
            (
                entry["callbackId"].as_str().unwrap_or_default().to_owned(),
                entry["state"]["status"].clone(),
            )
        })
        .collect()
}

async fn open_card(shared: &Arc<crate::core::Shared>) -> String {
    for _ in 0..200 {
        if let Some((id, _)) = cards(shared).into_iter().find(|(_, state)| state == "open") {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no browser profile card opened");
}

/// The agent's open through the authenticated hook, as Vibe runs it.
fn hook_open(
    shared: &Arc<crate::core::Shared>,
    profile: &str,
) -> tokio::task::JoinHandle<Result<(), String>> {
    let shared = shared.clone();
    let args = json!({"action":"open","profileId":profile});
    tokio::spawn(async move {
        tool_calls::permission(&shared, "s", "s", "call-1", "chrome_session", &args).await
    })
}

fn answer(shared: &Arc<crate::core::Shared>, id: &str, output: &Value) {
    tool_calls::answer_by(
        shared,
        crate::Origin::Ui,
        "s",
        id,
        output,
        AnsweredBy::Person,
    )
    .unwrap();
}

fn deny() -> Value {
    json!({"type":"approval","decision":{"type":"deny"}})
}

#[tokio::test]
async fn a_no_on_the_profile_card_tells_the_agent_to_stop_asking() {
    let core = core("card-agent-decline").await;
    let held = hook_open(&core.shared, "chrome");
    let id = open_card(&core.shared).await;
    answer(&core.shared, &id, &deny());
    let error = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains(DECLINED), "{error}");
    assert!(!error.contains("Explain why"), "{error}");
    assert!(!error.contains("Settings > Browser"), "{error}");
    core.shutdown().await;
}

#[tokio::test]
async fn a_second_profile_card_in_the_same_turn_is_refused_without_a_card() {
    let core = core("card-agent-second").await;
    let held = hook_open(&core.shared, "chrome");
    let id = open_card(&core.shared).await;
    answer(&core.shared, &id, &deny());
    tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    // The same turn asks again, with the shorthand or an exact profile: refused at once, no new card.
    for profile in ["chrome", "chrome:work"] {
        let again = tokio::time::timeout(Duration::from_secs(2), hook_open(&core.shared, profile))
            .await
            .expect("a second card in the same turn must not wait for an answer")
            .unwrap()
            .unwrap_err();
        assert!(again.contains(DECLINED), "{again}");
    }
    assert_eq!(cards(&core.shared).len(), 1, "{:?}", cards(&core.shared));
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    // The user's next message starts a new turn: the card may ask again.
    turn(&core.shared, "turn-2", vw_session::LoadKind::Replace);
    let held = hook_open(&core.shared, "chrome");
    let id = open_card(&core.shared).await;
    answer(
        &core.shared,
        &id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:personal"}),
    );
    tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn the_call_naming_another_profile_than_the_pick_is_told_which_profile_the_user_picked() {
    let core = core("card-agent-picked").await;
    // The agent asked for Personal; the person picked Work on the card.
    let held = hook_open(&core.shared, "chrome:personal");
    let id = open_card(&core.shared).await;
    answer(
        &core.shared,
        &id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:work"}),
    );
    tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let refused = super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:personal"}),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("The user picked Work (example.com) for this chat; use it"),
        "{refused}"
    );
    assert!(refused.contains("chrome:work"), "{refused}");
    for wrong in ["another chat", "another browser profile", "work@"] {
        assert!(!refused.contains(wrong), "{wrong}: {refused}");
    }
    // Person commands (the composer's tab picker) keep their own text, without the agent's profile ID.
    let person =
        super::profile_bindings::check(&core.shared.chrome_control, "s", "chrome:personal")
            .unwrap_err();
    assert!(person.contains("another browser profile"), "{person}");
    assert!(!person.contains("profileId"), "{person}");
    // The picked profile itself opens.
    super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    core.shutdown().await;
}

#[test]
fn the_profile_texts_tell_the_agent_to_open_with_the_browser_name_so_the_user_picks() {
    let wanted = "With several profiles and no default, open with the browser's name as profileId (\"chrome\" or \"edge\"), not a profile's ID; the user picks on a card.";
    assert!(
        include_str!("profile_agent.rs").contains(&wanted.replace('"', "\\\"")),
        "status profileSelection"
    );
    // The chrome_session description must equal the helper's own tools/list (the private catalog drift test), so it
    // stays as the helper ships it; the status text above carries the guidance.
    let list = json!(vw_agent_tools::chrome_tool_list());
    let spec = list
        .as_array()
        .unwrap()
        .iter()
        .find(|spec| spec["name"] == "chrome_session")
        .unwrap();
    assert!(
        spec["description"]
            .as_str()
            .unwrap()
            .contains("Use the exact profileId from chrome_status."),
        "{spec}"
    );
}

fn record(number: u32, folder: Option<&str>) -> super::profiles::Record {
    super::profiles::Record {
        owner: None,
        name: None,
        metadata_name: None,
        enabled: true,
        removed: false,
        folder: folder.map(std::path::PathBuf::from),
        deleted: false,
        browser_name: None,
        account: None,
        number: Some(number),
        paired_at: None,
        replaced_by: None,
    }
}

#[test]
fn a_numbered_profile_keeps_its_number_when_another_is_removed_or_its_folder_is_re_paired() {
    let mut saved = super::profiles::Profiles::default();
    saved
        .records
        .insert("chrome:a".into(), record(1, Some("/u/Default")));
    saved
        .records
        .insert("chrome:b".into(), record(2, Some("/u/Profile 2")));
    saved
        .records
        .insert("chrome:c".into(), record(3, Some("/u/Profile 3")));
    assert_eq!(saved.agent_name("chrome:c"), "Chrome profile 3");
    // Another profile is removed: the number stays with its profile.
    saved.records.get_mut("chrome:b").unwrap().removed = true;
    assert_eq!(saved.agent_name("chrome:c"), "Chrome profile 3");
    assert_eq!(saved.name("chrome:c"), "Chrome profile 3");
    // The same folder pairs again under a new identity, which retires the old one: same number.
    saved
        .records
        .insert("chrome:c2".into(), record(4, Some("/u/Profile 3")));
    assert!(saved.retired("chrome:c"));
    assert_eq!(saved.agent_name("chrome:c2"), "Chrome profile 3");
    assert_eq!(saved.agent_name("chrome:a"), "Chrome profile 1");
}

#[test]
fn a_profile_named_with_an_email_keeps_that_name_in_agent_text() {
    let mut saved = super::profiles::Profiles::default();
    let mut named = record(3, Some("/u/Profile 3"));
    named.browser_name = Some("test@example.com".into());
    named.account = Some("test@example.com".into());
    saved.records.insert("chrome:e".into(), named);
    saved.records.insert("chrome:a".into(), record(1, None));
    assert_eq!(saved.agent_name("chrome:e"), "test@example.com");
    // Only a profile without any name is numbered.
    assert_eq!(saved.agent_name("chrome:a"), "Chrome profile 1");
}
