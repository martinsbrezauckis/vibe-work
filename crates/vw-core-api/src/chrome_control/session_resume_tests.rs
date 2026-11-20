//! Retained chat sessions across turns: a person-stopped session is not reopened, a completed turn keeps the session
//! and releases control, and an unconfirmed or stopped resume never sends the requested action.
use serde_json::json;
use std::sync::Arc;

use super::session_lifetime_tests::{chat_with_session, helper, opens};

/// The person's Stop is not a turn end: that session is not reopened, and the refusal says what happened.
#[tokio::test]
async fn a_session_the_person_stopped_is_not_reopened() {
    let core = chat_with_session().await;
    let shared = &core.shared;
    let seen = super::test_support::answering(shared, helper("t2"));
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    crate::stops::stopped(shared, "s");
    let refused = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t2","action":"page.read","args":{}}),
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        "Not run: this Chrome browser session was stopped; open a new one with chrome_session."
    );
    assert_eq!(opens(&seen).len(), 1);
    core.shutdown().await;
}
#[tokio::test]
async fn completed_turn_retains_the_session_and_releases_control() {
    let core = chat_with_session().await;
    let seen = super::test_support::answering(&core.shared, helper("held"));
    super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    super::end_turn(&core.shared, "s");
    assert_eq!(
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .get("held")
            .map(String::as_str),
        Some("s")
    );
    tokio::task::yield_now().await;
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, a)| a["action"] == "stop")
    );
    // A completed turn must not expire an unanswered or skipped person question.
    let question = core.shared.asks.open(
        "user_input",
        Some("s".into()),
        vw_ask::Requester::Agent,
        vec![vw_ask::Choice::No],
        Some("question-hold".into()),
    );
    for skipped in [false, true] {
        if skipped {
            assert!(core.shared.asks.settle(
                &question.ask.id,
                vw_ask::Outcome::Refused {
                    by: vw_ask::AnsweredBy::Person,
                    note: None
                }
            ));
        }
        super::end_turn(&core.shared, "s");
        // A following owner call is the acknowledgement barrier for the queued control.
        core.shared
            .chrome_control
            .owner
            .call("chrome_status", json!({}))
            .await;
        let last = seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(_, args)| args["action"] == "idle")
            .unwrap()
            .1
            .clone();
        assert_eq!(last["awaitingPerson"], true, "skipped={skipped}: {last}");
    }
    core.shared.asks.clear_question_holds(Some("s"));
    super::end_turn(&core.shared, "s");
    core.shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    let last = seen
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(_, args)| args["action"] == "idle")
        .unwrap()
        .1
        .clone();
    assert_eq!(
        last["awaitingPerson"], false,
        "a new message releases the hold"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_unconfirmed_resume_never_sends_the_requested_action() {
    for state in ["connecting", "stopping", "disconnected"] {
        let core = chat_with_session().await;
        super::test_support::answering(&core.shared, helper("held"));
        super::gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"}),
        )
        .await
        .unwrap();
        super::end_turn(&core.shared, "s");
        let seen = super::test_support::answering(
            &core.shared,
            move |_, _| json!({"structuredContent":{"sessionId":"held","state":state}}),
        );
        let answer = super::gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"held","action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert!(answer.to_string().contains("SESSION_NOT_READY"), "{answer}");
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, _)| tool == "chrome_execute")
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn stop_during_resume_fences_the_action_and_does_not_rebind_the_session() {
    let core = chat_with_session().await;
    super::test_support::answering(&core.shared, helper("held"));
    super::gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    super::end_turn(&core.shared, "s");
    let weak = Arc::downgrade(&core.shared);
    let seen = super::test_support::answering(&core.shared, move |tool, args| {
        if tool == "chrome_session" && args["action"] == "resume" {
            crate::stops::stopped(&weak.upgrade().unwrap(), "s");
        }
        json!({"structuredContent":{"sessionId":"held","state":"active"}})
    });
    let answer = super::gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"held","action":"page.read","args":{}}),
    )
    .await;
    assert!(answer.is_err(), "{answer:?}");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    assert!(
        !core
            .shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("held")
    );
    core.shutdown().await;
}

/// Lowering a chat's access mode ends its browser session on purpose (the lower mode must not keep the higher one's
/// lease or installed rules), but never silently: the agent's next call says the mode was lowered and that the tabs
/// stay open, and the person gets a chat notice saying the same. Nothing closes the tabs.
/// PREDICT (before the change): the agent read only "this Chrome session was stopped; open a new one" (seen with
/// Safari on macOS after Full -> Ask: the agent reopened without knowing why) and the chat showed nothing.
#[tokio::test]
async fn a_lowered_mode_ends_the_session_and_says_why() {
    let core = chat_with_session().await;
    let shared = &core.shared;
    let seen = super::test_support::answering(shared, helper("t3"));
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    let mut events = core.subscribe(None);
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    super::check::mode_changed(shared, "s", Some("auto-approve"));
    let refused = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t3","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        refused,
        "Not run: this Chrome browser session ended because this chat's access mode was lowered; its tabs stay open. \
         Open a new one with chrome_session if the new mode allows the work."
    );
    let mut notices = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(300), events.next()).await
    {
        let value = serde_json::to_value(event).unwrap();
        if value["change"]["params"]["warning"]["code"] == "browser_mode_lowered" {
            notices.push(value["change"]["params"]["warning"]["message"].clone());
        }
    }
    assert_eq!(
        notices,
        vec![json!(
            "Your Chrome control ended because this chat's access mode was lowered. Its tabs stay open."
        )]
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "tabs.close" || args["action"] == "tabs.close_owned"),
        "no tab is closed"
    );
    // Raising the mode again ends nothing.
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    super::check::mode_changed(shared, "s", Some("ask"));
    core.shutdown().await;
}
