//! Several connected profiles and no default: the person picks this chat's profile on a card, not in the composer.
//!
//! PREDICT (written before the change): on the base an open in an unbound chat with two eligible Chrome profiles and
//! no default fails at once with "Several Chrome profiles are available ... Ask the user to select a profile in this
//! chat." No card opens, so `card()` never finds one and the first test times out; the composer selector it points
//! at no longer exists. After the change the open waits on an app-owned card that lists both profiles by name (the
//! account only in the card's screen detail), the person's pick binds the chat through the same person selection
//! as before, and the open continues in that profile. A pick that was not offered is refused, a No ends the open
//! with a text that names Settings > Browser, and no profile text tells the agent to choose "in this chat".
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and held calls must exist for the test to mean anything"
)]
use super::test_support;
use crate::ask::tool_calls;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use vw_ask::AnsweredBy;

fn status() -> Value {
    json!({"profiles":[
        {"profileId":"chrome:work","connected":true},
        {"profileId":"chrome:personal","connected":true}
    ]})
}

async fn core(name: &str) -> (crate::Core, Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
    core_with(name, status()).await
}

/// The fixture with the helper reporting `helper` as its status (which profiles are connected now).
async fn core_with(
    name: &str,
    helper: Value,
) -> (crate::Core, Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
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
    let reported = helper.clone();
    let seen = test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_status" {
            return json!({"structuredContent":reported});
        }
        if args["action"] == "open" {
            let task = format!("task-{}", args["profileId"].as_str().unwrap_or_default());
            return json!({"structuredContent":{"sessionId":task,"profileId":args["profileId"],"state":"active"}});
        }
        json!({"structuredContent":{}})
    });
    test_support::observe_status(&core.shared, &status());
    test_support::observe_status(&core.shared, &helper);
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
    (core, seen)
}

/// The open profile card of chat `s`, once the held open has published it.
async fn card(shared: &Arc<crate::core::Shared>) -> Value {
    for _ in 0..200 {
        let state = crate::sessions::snapshot(shared, "s").unwrap();
        let entry = serde_json::to_value(&state)
            .unwrap()
            .pointer("/state/history")
            .and_then(Value::as_array)
            .and_then(|history| {
                history
                    .iter()
                    .find(|entry| {
                        entry["detail"]["effect"]["kind"] == "browserProfile"
                            && entry["state"]["status"] == "open"
                    })
                    .cloned()
            });
        if let Some(entry) = entry {
            return entry;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no browser profile card opened");
}

/// The profile card `id` of chat `s` as the transcript shows it now.
fn answered(shared: &Arc<crate::core::Shared>, id: &str) -> Value {
    let state = serde_json::to_value(crate::sessions::snapshot(shared, "s").unwrap()).unwrap();
    state
        .pointer("/state/history")
        .and_then(Value::as_array)
        .and_then(|history| history.iter().find(|entry| entry["id"] == id).cloned())
        .unwrap()
}

fn open(shared: &Arc<crate::core::Shared>) -> tokio::task::JoinHandle<Result<Value, String>> {
    let shared = shared.clone();
    tokio::spawn(async move {
        super::gateway::call(
            &shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"chrome"}),
        )
        .await
    })
}

fn answer(shared: &Arc<crate::core::Shared>, id: &str, output: &Value) -> Result<(), String> {
    tool_calls::answer_by(
        shared,
        crate::Origin::Ui,
        "s",
        id,
        output,
        AnsweredBy::Person,
    )
    .map(|_| ())
    .map_err(|error| error.message)
}

fn selected(core: &crate::Core) -> Option<String> {
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .get("s")
        .cloned()
}

#[tokio::test]
async fn the_person_picks_the_profile_on_a_card_and_the_open_continues_there() {
    let (core, seen) = core("profile-choice-card").await;
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    let choices = entry["detail"]["effect"]["choices"].as_array().unwrap();
    let names: Vec<_> = choices.iter().map(|row| row["name"].clone()).collect();
    assert!(
        names.contains(&json!("Work")) && names.contains(&json!("Personal")),
        "{entry}"
    );
    // The account is on the card for the person; the card is screen state, not a tool result.
    assert!(
        choices
            .iter()
            .any(|row| row["account"] == "work@example.com"),
        "{entry}"
    );
    let id = entry["callbackId"].as_str().unwrap();
    answer(
        &core.shared,
        id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:personal"}),
    )
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap();
    let text = result.unwrap().to_string();
    assert!(!text.contains("example.com"), "{text}");
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "open" && args["profileId"] == "chrome:personal"),
        "the open runs in the picked profile"
    );
    // The answered card keeps which profile was picked, so its audit row can name it.
    assert_eq!(
        answered(&core.shared, id)["detail"]["effect"]["picked"],
        "chrome:personal"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_profile_the_card_did_not_offer_is_refused() {
    let (core, _) = core("profile-choice-not-offered").await;
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    let id = entry["callbackId"].as_str().unwrap();
    let refused = answer(
        &core.shared,
        id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:other"}),
    )
    .unwrap_err();
    assert!(refused.contains("not offered"), "{refused}");
    let driven = tool_calls::answer_by(
        &core.shared,
        crate::Origin::DebugApi,
        "s",
        id,
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:work"}),
        AnsweredBy::DebugApi,
    )
    .map(|_| ())
    .map_err(|error| error.message)
    .unwrap_err();
    assert!(driven.contains("requires the user"), "{driven}");
    let missing = answer(
        &core.shared,
        id,
        &json!({"type":"approval","decision":{"type":"approve"}}),
    )
    .unwrap_err();
    assert!(missing.contains("not offered"), "{missing}");
    answer(
        &core.shared,
        id,
        &json!({"type":"approval","decision":{"type":"deny"}}),
    )
    .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    // A No tells the agent to leave the browser alone in this chat (`profile_card_agent_tests`).
    assert!(error.contains("chose not to use Chrome"), "{error}");
    assert_eq!(selected(&core), None);
    core.shutdown().await;
}

#[test]
fn no_profile_text_tells_the_agent_to_choose_in_this_chat() {
    // The composer has no profile control: every recovery names the card or Settings > Browser.
    for (file, source) in [
        ("profile_presence.rs", include_str!("profile_presence.rs")),
        ("profile_bindings.rs", include_str!("profile_bindings.rs")),
        ("profile_selection.rs", include_str!("profile_selection.rs")),
        ("profile_seeding.rs", include_str!("profile_seeding.rs")),
        ("profile_agent.rs", include_str!("profile_agent.rs")),
        ("commands.rs", include_str!("commands.rs")),
        ("profiles.rs", include_str!("profiles.rs")),
    ] {
        for line in source.lines().filter(|line| line.contains('"')) {
            let lower = line.to_lowercase();
            assert!(
                !(lower.contains("in this chat") || lower.contains("in the chat"))
                    || !(lower.contains("select") || lower.contains("choose")),
                "{file}: {line}"
            );
        }
    }
}

#[tokio::test]
async fn a_chat_whose_profile_the_browser_deleted_asks_for_another_on_the_card() {
    let (core, _) = core("profile-choice-deleted").await;
    // An earlier pick bound the chat to Work; then the browser deleted that profile.
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
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
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut("chrome:work")
        .unwrap()
        .deleted = true;
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    assert_eq!(
        entry["detail"]["effect"]["choices"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "only the connected profile is offered: {entry}"
    );
    assert_eq!(entry["detail"]["effect"]["choices"][0]["name"], "Personal");
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
    core.shutdown().await;
}

#[tokio::test]
async fn the_card_waits_in_the_authenticated_hook_before_the_open_runs() {
    let (core, seen) = core("profile-choice-hook").await;
    let shared = core.shared.clone();
    // The hook is where person cards wait without a tool timeout; the open itself then runs in the picked profile.
    let hook = tokio::spawn(async move {
        tool_calls::permission(
            &shared,
            "s",
            "s",
            "call-1",
            "chrome_session",
            &json!({"action":"open","profileId":"chrome:work"}),
        )
        .await
    });
    let entry = card(&core.shared).await;
    answer(
        &core.shared,
        entry["callbackId"].as_str().unwrap(),
        &json!({"type":"approval","decision":{"type":"approve"},"profileId":"chrome:personal"}),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), hook)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(selected(&core).as_deref(), Some("chrome:personal"));
    // The agent's exact other profile is refused with the chat's next step; the shorthand opens the picked one.
    let refused = super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:work"}),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("The user picked Personal for this chat"),
        "{refused}"
    );
    open(&core.shared).await.unwrap().unwrap();
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "open" && args["profileId"] == "chrome:personal")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn the_hook_asks_again_when_the_browser_deleted_the_chats_profile() {
    let (core, _) = core("profile-choice-hook-deleted").await;
    let hook = |shared: Arc<crate::core::Shared>| {
        tokio::spawn(async move {
            tool_calls::permission(
                &shared,
                "s",
                "s",
                "call-1",
                "chrome_session",
                &json!({"action":"open","profileId":"chrome"}),
            )
            .await
        })
    };
    let held = hook(core.shared.clone());
    let entry = card(&core.shared).await;
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
    core.shared
        .chrome_control
        .profiles
        .lock()
        .unwrap()
        .records
        .get_mut("chrome:work")
        .unwrap()
        .deleted = true;
    let held = hook(core.shared.clone());
    let entry = card(&core.shared).await;
    assert_eq!(entry["detail"]["effect"]["choices"][0]["name"], "Personal");
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
    core.shutdown().await;
}

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

/// A decline on the profile card is the person's answer for this chat: the agent is told not to open the browser
/// again or ask about its profiles unless the user asks, and the step stays not done.
///
/// PREDICT (written before the change): fails. The decline returns the generic text "Not run: the user declined
/// this. … Explain why it is needed for the user's delegated task.", which invites the agent to retry, and the
/// real runs showed the card coming back up to three times in one turn; it has no "Do not open Chrome again".
#[tokio::test]
async fn a_decline_tells_the_agent_not_to_ask_again() {
    let (core, seen) = core("profile-choice-decline").await;
    let held = open(&core.shared);
    let entry = card(&core.shared).await;
    answer(
        &core.shared,
        entry["callbackId"].as_str().unwrap(),
        &json!({"type":"approval","decision":{"type":"deny"}}),
    )
    .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(10), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("not to use Chrome"), "{error}");
    assert!(error.contains("Do not open it again"), "{error}");
    assert!(error.contains("not done"), "{error}");
    assert!(!error.contains("Explain why"), "{error}");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "open")
    );
    core.shutdown().await;
}
