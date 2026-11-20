//! Stops across turn boundaries and lost connections: no Stop leaks into the next turn, one interrupt per turn, a closed window is not the person's Stop.
use super::person_stop_tests::{fixture, runtime, status, turn};
use super::{agent_browser::Browser, gateway, owner};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

async fn next_turn(kind: &str, delay: u64) {
    let core = fixture(&format!("browser-stop-next-{kind}-{delay}"), "chrome:p").await;
    let old_poll = Instant::now();
    match kind {
        "person" => status(&core, "stopping", Instant::now()),
        "agent" => {
            gateway::call(
                &core.shared,
                "a",
                "chrome_session",
                json!({"action":"stop","sessionId":"task-a"}),
            )
            .await
            .unwrap();
        }
        _ => {}
    }
    super::end_turn(&core.shared, "a");
    turn(&core, "a", "one", false);
    // A poll already in flight, then a fresh poll before the helper processes cleanup, can both say active.
    status(&core, "active", old_poll);
    status(&core, "active", Instant::now());
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(delay)).await;
    tokio::time::resume();
    turn(&core, "a", "two", true);
    status(&core, "stopped", Instant::now());
    let reply = gateway::call(
        &core.shared,
        "a",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:p"}),
    )
    .await;
    assert!(
        reply.is_ok(),
        "{kind} stop leaked into next turn after {delay}s: {reply:?}"
    );
    assert!(
        !core
            .shared
            .chrome_control
            .stopped_by_person("a", Browser::Chrome, Some("two"))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn cleanup_stop_never_becomes_a_person_stop_in_the_next_turn() {
    for delay in [0, 19] {
        next_turn("cleanup", delay).await;
    }
}
#[tokio::test]
async fn agent_stop_never_becomes_a_person_stop_in_the_next_turn() {
    for delay in [0, 19] {
        next_turn("agent", delay).await;
    }
}
#[tokio::test]
async fn person_stop_cannot_leak_into_the_next_turn() {
    for delay in [0, 19] {
        next_turn("person", delay).await;
    }
}

#[tokio::test]
async fn an_idle_chat_has_no_turn_to_interrupt() {
    let core = fixture("person-stop-idle", "chrome:p").await;
    turn(&core, "a", "one", false);
    status(&core, "stopped", Instant::now());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(crate::stops::generation(&core.shared, "a"), 0);
    turn(&core, "a", "two", true);
    assert!(
        gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:p"})
        )
        .await
        .is_ok()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_delayed_interrupt_does_not_take_the_next_turn() {
    let core = fixture("browser-delayed-interrupt", "chrome:p").await;
    let calls = runtime(&core);
    status(&core, "stopping", Instant::now());
    // No yield: the next turn starts before the interrupt task gets its first poll.
    turn(&core, "a", "two", true);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(crate::stops::generation(&core.shared, "a"), 0);
    assert!(
        gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:p"})
        )
        .await
        .is_ok()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_stop_status_for_an_older_turn_cannot_fence_the_current_turn() {
    let core = fixture("browser-delayed-status", "chrome:p").await;
    turn(&core, "a", "two", true);
    status(&core, "stopped", Instant::now());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(crate::stops::generation(&core.shared, "a"), 0);
    assert!(
        gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:p"})
        )
        .await
        .is_ok()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_agent_stop_survives_an_active_poll_before_acknowledgement() {
    let core = fixture("browser-agent-stop-pending", "chrome:p").await;
    let weak = Arc::downgrade(&core.shared);
    core.shared.chrome_control.owner.answering(move |_, _| {
        let shared = weak.upgrade().unwrap();
        for state in ["active", "stopping", "stopped"] {
            owner::update_work_for_test(
                &shared,
                &json!({"structuredContent":{"sessions":[
                    {"sessionId":"task-a","state":state}, {"sessionId":"task-b","state":"active"}
                ]}}),
            );
        }
        json!({"structuredContent":{"sessionId":"task-a","state":"stopped"}})
    });
    gateway::call(
        &core.shared,
        "a",
        "chrome_session",
        json!({"action":"stop","sessionId":"task-a"}),
    )
    .await
    .unwrap();
    assert_eq!(crate::stops::generation(&core.shared, "a"), 0);
    assert!(
        !core
            .shared
            .chrome_control
            .stopped_by_person("a", Browser::Chrome, Some("one"))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_inflight_reply_keeps_the_person_stop_reason_after_turn_end() {
    let core = fixture("browser-inflight-stop-reason", "chrome:p").await;
    let weak = Arc::downgrade(&core.shared);
    core.shared.chrome_control.owner.answering(move |_, _| {
        let shared = weak.upgrade().unwrap();
        owner::update_work_for_test(
            &shared,
            &json!({"structuredContent":{"sessions":[{"sessionId":"task-a","state":"stopped"}]}}),
        );
        super::end_turn(&shared, "a");
        json!({"structuredContent":{"error":{"code":"CONTROL_SUSPENDED"}}})
    });
    let reason = gateway::call(
        &core.shared,
        "a",
        "chrome_execute",
        json!({"sessionId":"task-a","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason, super::person_stop::refusal(Browser::Chrome));
    core.shutdown().await;
}

#[cfg(feature = "your-edge")]
#[tokio::test(start_paused = true)]
async fn stops_of_both_browsers_share_one_turn_interrupt() {
    let core = fixture("browser-both-stop", "chrome:p").await;
    core.shared.chrome_control.work.lock().unwrap().open(
        "edge-task",
        "a",
        "active",
        "edge:p",
        Some("one"),
    );
    core.shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("edge-task".into(), "a".into());
    let calls = runtime(&core);
    owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"sessions":[
            {"sessionId":"task-a","state":"stopped"}, {"sessionId":"edge-task","state":"stopped"}, {"sessionId":"task-b","state":"active"}
        ]}}),
    );
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.lock().unwrap().len(), 1);
    core.shutdown().await;
}

/// A session the helper ended because its browser connection was lost (its owned window closed:
/// `suspensionReason` `ADAPTER_DISCONNECTED`) still ends the turn once, but the chat never says the person stopped it.
async fn connection_end_is_not_attributed_to_the_person(profile: &str, browser: &str) {
    let core = fixture(&format!("connection-ended-{browser}"), profile).await;
    let mut events = core.subscribe(None);
    let calls = runtime(&core);
    owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"sessions":[
            {"sessionId":"task-a","state":"stopped","suspensionReason":"ADAPTER_DISCONNECTED"},
            {"sessionId":"task-b","state":"active","suspensionReason":null}
        ]}}),
    );
    let notice = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = serde_json::to_value(events.next().await.unwrap()).unwrap();
            if value["change"]["kind"] == "notice" {
                break value;
            }
        }
    })
    .await
    .expect("the ended browser session must leave a notice in its chat");
    assert_eq!(notice["change"]["sessionId"], "a");
    assert_eq!(
        notice["change"]["params"]["warning"]["message"],
        format!("Stopped in Your {browser}: the browser connection ended")
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while core.shared.store.view("a").unwrap().state["turns"][0]["status"] != "interrupted" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a lost browser connection still ends the turn");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![json!({"sessionId":"a","expectedTurnId":"one"})]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_connection_end_is_not_attributed_to_the_person() {
    connection_end_is_not_attributed_to_the_person("chrome:p", "Chrome").await;
}

#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn owned_firefox_window_end_is_not_attributed_to_the_person() {
    connection_end_is_not_attributed_to_the_person("firefox:agent", "Firefox").await;
}

/// The helper's confirmation of the agent frame a call shows first.
fn frame(args: &serde_json::Value) -> serde_json::Value {
    json!({"structuredContent":{"sessionId":args["sessionId"],"frame":{"state":args["state"]}}})
}

/// PREDICT (fails before the fix): the person's Stop in the browser reaches Vibe Work only with the next status poll
/// (every 2 s). An agent call sent between the Stop and that poll gets the helper's `CONTROL_SUSPENDED` "Session start
/// must be acknowledged", and the gateway passed it through as the call's answer: the agent read helper words with no
/// next step and the person's line said "Paused". The suspended answer must make the core read the helper's status
/// once more, so the Stop it already holds decides the answer: the person-stop refusal.
#[tokio::test]
async fn a_suspended_answer_before_the_next_poll_reads_the_persons_stop() {
    let core = fixture("browser-suspended-before-poll", "chrome:p").await;
    let calls = core.shared.chrome_control.owner.answering(|name, args| {
        if args["action"] == "frame" {
            frame(args)
        } else if name == "chrome_status" {
            json!({"structuredContent":{"sessions":[
                {"sessionId":"task-a","state":"stopped","suspensionReason":"STOP_REQUESTED"},
                {"sessionId":"task-b","state":"active"}
            ]}})
        } else {
            let error =
                json!({"code":"CONTROL_SUSPENDED","message":"Session start must be acknowledged"});
            json!({"structuredContent":{"error":error,"hostError":error}})
        }
    });
    let reason = gateway::call(
        &core.shared,
        "a",
        "chrome_execute",
        json!({"sessionId":"task-a","action":"page.read","args":{}}),
    )
    .await
    .unwrap_err();
    assert_eq!(reason, super::person_stop::refusal(Browser::Chrome));
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|(name, _)| name == "chrome_status"),
        "the suspended answer did not read the helper's status"
    );
    core.shutdown().await;
}

/// A suspended session that nobody stopped (still connecting, or already ended) gets Vibe Work's own words with the
/// next step, never the helper's "Session start must be acknowledged".
#[tokio::test]
async fn a_suspended_answer_without_a_stop_gives_the_fixed_next_step() {
    let core = fixture("browser-suspended-no-stop", "chrome:p").await;
    core.shared.chrome_control.owner.answering(|name, args| {
        if args["action"] == "frame" {
            frame(args)
        } else if name == "chrome_status" {
            json!({"structuredContent":{"sessions":[
                {"sessionId":"task-a","state":"active"}, {"sessionId":"task-b","state":"active"}
            ]}})
        } else {
            let error =
                json!({"code":"CONTROL_SUSPENDED","message":"Session start must be acknowledged"});
            json!({"structuredContent":{"error":error,"hostError":error}})
        }
    });
    let reply = gateway::call(
        &core.shared,
        "a",
        "chrome_execute",
        json!({"sessionId":"task-a","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    let error = &reply["vwMcpResult"]["structuredContent"]["error"];
    assert_eq!(error["code"], "CONTROL_SUSPENDED", "{reply}");
    assert_eq!(
        error["message"],
        vw_vocabulary::texts::AGENT_BROWSER_NOT_ACTIVE,
        "{reply}"
    );
    core.shutdown().await;
}
