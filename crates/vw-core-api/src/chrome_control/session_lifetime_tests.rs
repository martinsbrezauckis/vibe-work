//! Chat lifecycle regressions through the checked gateway and a recording helper.
//! Retains the old ownership, stop and stale-reply tests while checking same-session follow-ups.
use serde_json::{Value, json};
use std::sync::Arc;

async fn chat_with_session() -> crate::Core {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = format!(
        "chrome-reopen-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let core = crate::offline_core::start_with(&name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await;
    let shared = &core.shared;
    for chat in ["s", "o"] {
        let _ = shared
            .store
            .load(
                json!({"session":{"id":chat},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            chat,
            &json!({"agentName":"auto-approve"}),
        );
    }
    core
}

fn helper(opened: &'static str) -> impl Fn(&str, &Value) -> Value + Send + 'static {
    move |tool, args| {
        if tool == "chrome_session" && args["action"] == "open" {
            json!({"isError":false,"content":[{"type":"text","text":"opened"}],
                "structuredContent":{"sessionId":opened,"state":"active","profileId":"p"}})
        } else if tool == "chrome_session" && args["action"] == "resume" {
            json!({"structuredContent":{"sessionId":args["sessionId"],"state":"active"}})
        } else {
            json!({"isError":false,"content":[{"type":"text","text":"done"}],
                "structuredContent":{"operationId":"op","status":"completed","sessionId":args["sessionId"],"data":{}}})
        }
    }
}

fn opens(seen: &Arc<std::sync::Mutex<Vec<(String, Value)>>>) -> Vec<Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(t, a)| t == "chrome_session" && a["action"] == "open")
        .map(|(_, a)| a.clone())
        .collect()
}

#[tokio::test]
async fn a_completed_turn_keeps_the_same_session_for_reads_and_batches() {
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
    super::end_turn(shared, "s");
    for _ in 0..2 {
        let answer = super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert_eq!(
            answer["vwMcpResult"]["structuredContent"]["status"],
            "completed"
        );
        assert!(answer["vwMcpResult"]["structuredContent"]["sessionReopened"].is_null());
    }
    assert_eq!(opens(&seen).len(), 1);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(t, a)| t == "chrome_execute" && a["sessionId"] == "t2")
    );
    // A batch with the ended id runs too.
    let batch = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t2","steps":[{"action":"page.read","args":{}}]}),
    )
    .await
    .unwrap();
    assert_eq!(
        batch["vwMcpResult"]["structuredContent"]["status"], "completed",
        "{batch}"
    );
    // Another chat never gets this chat's session, ended or not.
    let other = super::gateway::call(
        shared,
        "o",
        "chrome_execute",
        json!({"sessionId":"t2","action":"page.read","args":{}}),
    )
    .await;
    assert_eq!(
        other.unwrap_err(),
        "Not run: this Chrome session belongs to another chat."
    );
    // An id no chat ever had says so (Mistral passed a made-up "default" in T34/T35).
    let unknown = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"default","action":"page.read","args":{}}),
    )
    .await;
    assert_eq!(
        unknown.unwrap_err(),
        "Not run: no browser session with this id; open one with chrome_session."
    );
    assert_eq!(opens(&seen).len(), 1);
    // Archiving a quiet chat ends its retained browser authority; restoring it requires a fresh session.
    crate::session_end::archive_decision(shared, &json!({"sessionId":"s","archived":true})).await;
    let archived = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t2","action":"page.read","args":{}}),
    )
    .await;
    assert!(
        archived.is_err(),
        "archived chat retained browser authority: {archived:?}"
    );
    core.shutdown().await;
}

/// A status poll can report the stop before the helper replies to the agent's own request.
#[tokio::test]
async fn the_agents_stop_allows_a_new_session_in_the_same_turn() {
    let core = chat_with_session().await;
    let shared = &core.shared;
    super::test_support::answering(shared, helper("t2"));
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    let weak = Arc::downgrade(shared);
    let seen = shared.chrome_control.owner.answering(move |tool, args| {
        if tool == "chrome_session" && args["action"] == "stop" {
            let shared = weak.upgrade().unwrap();
            for state in ["stopping", "stopped"] {
                super::owner::update_work_for_test(
                    &shared,
                    &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":state}]}}),
                );
            }
        }
        helper("t3")(tool, args)
    });
    // Another chat cannot mark this session as an asked stop or reach the helper.
    assert!(
        super::gateway::call(
            shared,
            "o",
            "chrome_session",
            json!({"action":"stop","sessionId":"t2"}),
        )
        .await
        .is_err()
    );
    assert!(seen.lock().unwrap().is_empty());
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"stop","sessionId":"t2"}),
    )
    .await
    .unwrap();
    let reopened = super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap_or_else(|reason| panic!("agent stop blocked the same turn: {reason}"));
    assert_eq!(
        reopened["vwMcpResult"]["structuredContent"]["sessionId"],
        "t3"
    );
    assert_eq!(opens(&seen).len(), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn a_refused_agent_stop_recovers_on_the_next_poll() {
    let core = chat_with_session().await;
    let shared = &core.shared;
    super::test_support::answering(shared, helper("t2"));
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"isError":true,"structuredContent":{"error":{"code":"BUSY"}}}));
    let refused = super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"stop","sessionId":"t2"}),
    )
    .await
    .unwrap();
    assert_eq!(refused["vwMcpResult"]["isError"], true);
    assert_eq!(
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("t2", "s"),
        Some("stopping")
    );
    super::owner::update_work_for_test(
        shared,
        &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"active"}]}}),
    );
    assert_eq!(
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("t2", "s"),
        Some("active")
    );
    assert_eq!(
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .get("t2")
            .map(String::as_str),
        Some("s")
    );
    assert!(!shared.chrome_control.stopped_by_person(
        "s",
        super::agent_browser::Browser::Chrome,
        None
    ));
    // Recovery must also restore detection of a later real Stop.
    super::owner::update_work_for_test(
        shared,
        &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopping"}]}}),
    );
    assert!(shared.chrome_control.stopped_by_person(
        "s",
        super::agent_browser::Browser::Chrome,
        None
    ));
    core.shutdown().await;
}

/// After the person's Stop (the toolbar popup: the helper reports the
/// chat's active session stopped without Vibe Work asking), every Chrome call of that chat that would act is
/// refused for the rest of the turn, opening a new session included; the next turn starts normally.
/// PREDICT (before the change): the open after the Stop runs and the agent goes on.
#[tokio::test]
async fn the_persons_stop_holds_for_the_rest_of_the_turn() {
    let stopped = format!(
        "Not run: the user pressed Stop in {}. Do not use the browser again in this turn; tell them what you were doing and wait for their answer.",
        vw_vocabulary::texts::CHROME_NAME
    );
    let core = chat_with_session().await;
    let shared = &core.shared;
    let _seen = super::test_support::answering(shared, helper("t2"));
    super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"p"}),
    )
    .await
    .unwrap();
    // Stop is acknowledged asynchronously: a poll sees stopping before a later poll sees stopped.
    // The old code forgot who requested Stop as soon as the first poll changed the displayed state.
    super::owner::update_work_for_test(
        shared,
        &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopping"}]}}),
    );
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(40)).await;
    tokio::time::resume();
    super::owner::update_work_for_test(
        shared,
        &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopped"}]}}),
    );
    let seen = super::test_support::answering(shared, helper("t3"));
    for (tool, args) in [
        ("chrome_session", json!({"action":"open","profileId":"p"})),
        (
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        ),
        (
            "chrome_execute",
            json!({"sessionId":"t2","steps":[{"action":"page.read","args":{}}]}),
        ),
    ] {
        assert_eq!(
            super::gateway::call(shared, "s", tool, args)
                .await
                .unwrap_err(),
            stopped,
            "{tool}"
        );
    }
    assert!(
        opens(&seen).is_empty(),
        "a new helper session must never be requested"
    );
    // Reading the session stays possible; another chat is not affected.
    assert!(
        super::gateway::call(
            shared,
            "o",
            "chrome_session",
            json!({"action":"open","profileId":"p"})
        )
        .await
        .is_ok()
    );
    // The turn ends: the next turn starts normally, with its own new helper ID.
    super::test_support::answering(shared, helper("t4"));
    super::end_turn(shared, "s");
    assert!(
        super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":"p"})
        )
        .await
        .is_ok()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_explicit_stop_revokes_the_retained_session() {
    for app_stop in [false, true] {
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
        super::end_turn(shared, "s");
        drop(seen);
        let seen = super::test_support::answering(shared, helper("t3"));
        let resumed = super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert_eq!(
            resumed["vwMcpResult"]["structuredContent"]["sessionId"],
            "t2"
        );
        if app_stop {
            crate::stops::stopped(shared, "s");
        } else {
            super::owner::update_work_for_test(
                shared,
                &json!({"structuredContent":{"sessions":[{"sessionId":"t2","state":"stopped"}]}}),
            );
        }
        super::end_turn(shared, "s");
        let later = super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t2","action":"page.read","args":{}}),
        )
        .await;
        assert!(later.is_err(), "app_stop={app_stop}: {later:?}");
        assert_eq!(opens(&seen).len(), 0, "app_stop={app_stop}");
        // A new session of the chat in a later turn is fine.
        assert!(
            super::gateway::call(
                shared,
                "s",
                "chrome_session",
                json!({"action":"open","profileId":"p"})
            )
            .await
            .is_ok()
        );
        core.shutdown().await;
    }
}

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
        "Not run: this Chrome session was stopped; open a new one with chrome_session."
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

#[path = "session_combined_tests.rs"]
mod combined;
