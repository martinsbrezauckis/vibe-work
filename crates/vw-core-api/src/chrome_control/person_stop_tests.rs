//! Helper Stop regressions through the app observer and gateway; isolated cores, no browser or desktop.
use super::{agent_browser::Browser, gateway, owner, test_support};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

async fn fixture(name: &str, profile: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    for chat in ["a", "b"] {
        turn(&core, chat, "one", true);
        core.shared.access.observe_request(
            "session/agent/update",
            chat,
            &json!({"agentName":"auto-approve"}),
        );
        core.shared.chrome_control.work.lock().unwrap().open(
            &format!("task-{chat}"),
            chat,
            "active",
            profile,
            Some("one"),
        );
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert(format!("task-{chat}"), chat.into());
    }
    *core
        .shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap() = (crate::chrome_control::PairingState::Paired, None);
    test_support::answering(
        &core.shared,
        |_, args| json!({"structuredContent":{"sessionId":"new-task","profileId":args["profileId"],"state":"active"}}),
    );
    core
}

fn turn(core: &crate::Core, chat: &str, id: &str, running: bool) {
    let _ = core.shared.store.load(json!({
        "session":{"id":chat,"status":{"type":if running {"running"} else {"idle"},"activeTurnId":running.then_some(id)}},
        "history":[],"turns":[{"id":id,"status":if running {"in_progress"} else {"completed"}}]
    }), vw_session::LoadKind::New).unwrap();
}

fn status(core: &crate::Core, state: &str, started: Instant) {
    owner::update_work(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"sessions":[
            {"sessionId":"task-a","state":state}, {"sessionId":"task-b","state":"active"}
        ]}}),
        started,
    );
}

#[tokio::test]
async fn a_new_turn_never_receives_the_previous_stopped_session() {
    for fresh in [true, false] {
        let core = fixture(&format!("browser-stopped-open-{fresh}"), "chrome:p").await;
        status(&core, "stopped", Instant::now());
        super::end_turn(&core.shared, "a");
        turn(&core, "a", "two", true);
        let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = opens.clone();
        let seen = core
            .shared
            .chrome_control
            .owner
            .answering(move |tool, args| {
                if tool == "chrome_status" {
                    return json!({"structuredContent":{}});
                }
                assert_eq!(tool, "chrome_session");
                assert_eq!(args["action"], "open");
                let current = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let ready = fresh && current >= 2;
                json!({"structuredContent":{"sessionId":if ready {"fresh"} else {"task-a"},
                "profileId":"chrome:p","state":if ready {"active"} else {"stopped"}}})
            });
        let result = gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:p"}),
        )
        .await;
        if fresh {
            assert_eq!(
                result.unwrap()["vwMcpResult"]["structuredContent"]["sessionId"],
                "fresh"
            );
        } else {
            assert!(result.unwrap_err().contains("stopped session"));
        }
        assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .map(|(tool, _)| tool.as_str())
                .collect::<Vec<_>>(),
            [
                "chrome_status",
                "chrome_session",
                "chrome_session",
                "chrome_session"
            ]
        );
        assert!(
            !core
                .shared
                .chrome_control
                .tasks
                .lock()
                .unwrap()
                .contains_key("task-a")
        );
        core.shutdown().await;
    }
}

/// Recovery only retries a known terminal session from an earlier turn, never a possibly effective open.
#[tokio::test]
async fn stopped_open_recovery_never_replays_an_unknown_or_newly_stopped_open() {
    for unknown in [true, false] {
        let core = fixture(&format!("browser-open-no-replay-{unknown}"), "chrome:p").await;
        status(&core, "stopped", Instant::now());
        super::end_turn(&core.shared, "a");
        turn(&core, "a", "two", true);
        let seen = core.shared.chrome_control.owner.answering(move |tool, _| {
            if tool == "chrome_status" {
                return json!({"structuredContent":{}});
            }
            json!({"structuredContent":{"sessionId":if unknown {"task-a"} else {"new-stopped"},
                "state":"stopped","status":if unknown {"unknown"} else {"failed"}}})
        });
        let result = gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":"open","profileId":"chrome:p"}),
        )
        .await;
        if unknown {
            assert_eq!(
                result.unwrap()["vwMcpResult"]["structuredContent"]["status"],
                "unknown"
            );
        } else {
            assert!(result.unwrap_err().contains("stopped session"));
        }
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .map(|(tool, _)| tool.as_str())
                .collect::<Vec<_>>(),
            ["chrome_status", "chrome_session"]
        );
        assert!(
            !core
                .shared
                .chrome_control
                .tasks
                .lock()
                .unwrap()
                .values()
                .any(|chat| chat == "a")
        );
        core.shutdown().await;
    }
}

/// An active reply with a terminal lease id may have had an effect; preserve that uncertainty.
#[tokio::test]
async fn reusing_an_ended_id_as_active_is_not_replayed_or_claimed_not_run() {
    let core = fixture("browser-reused-active-open", "chrome:p").await;
    status(&core, "stopped", Instant::now());
    super::end_turn(&core.shared, "a");
    turn(&core, "a", "two", true);
    let seen = core.shared.chrome_control.owner.answering(|tool, _| {
        if tool == "chrome_status" {
            json!({"structuredContent":{}})
        } else {
            json!({"structuredContent":{"sessionId":"task-a","state":"active"}})
        }
    });
    let reason = gateway::call(
        &core.shared,
        "a",
        "chrome_session",
        json!({"action":"open","profileId":"chrome:p"}),
    )
    .await
    .unwrap_err();
    assert!(reason.contains("reused an ended session"));
    assert!(!reason.contains("Not run"));
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .map(|(tool, _)| tool.as_str())
            .collect::<Vec<_>>(),
        ["chrome_status", "chrome_session"]
    );
    core.shutdown().await;
}

/// The app uses its real gate, channel transport and pump; the peer answers only Vibe's interrupt method.
fn runtime(core: &crate::Core) -> Arc<Mutex<Vec<Value>>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let calls = Arc::new(Mutex::new(Vec::new()));
    for (index, chat) in ["a", "b"].into_iter().enumerate() {
        let (client, server) = tokio::io::duplex(16384);
        let (read, write) = tokio::io::split(client);
        let connection =
            vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
        core.shared
            .channels
            .bind_test_peer(chat, connection, (0, index as u64 + 1));
        let calls = calls.clone();
        tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                if request["method"] == "session/turn/queue/remove" {
                    assert_eq!(request["params"]["sessionId"], "a");
                    assert_eq!(request["params"]["queueItemId"], "queued");
                    let reply = json!({"jsonrpc":"2.0","id":request["id"],"result":{}});
                    write
                        .write_all(format!("{reply}\n").as_bytes())
                        .await
                        .unwrap();
                    continue;
                }
                assert_eq!(request["method"], "turn/interrupt", "{request}");
                calls.lock().unwrap().push(request["params"].clone());
                let reply = json!({"jsonrpc":"2.0","id":request["id"],"result":{"accepted":true}});
                let completed = json!({"jsonrpc":"2.0","method":"turn/completed","params":{
                    "sessionId":chat,"eventId":1,"turn":{"id":request["params"]["expectedTurnId"],"status":"interrupted"}
                }});
                let queued = json!({"jsonrpc":"2.0","method":"turn/queueUpdated","params":{
                    "sessionId":chat,"eventId":2,"queue":{"paused":true,"items":[{"id":"queued","message":[{"type":"text","text":"Next message"}]}]}
                }});
                for frame in [reply, completed, queued] {
                    write
                        .write_all(format!("{frame}\n").as_bytes())
                        .await
                        .unwrap();
                }
            }
        });
    }
    calls
}

async fn interrupted_once(profile: &str, browser: &str) {
    let core = fixture(&format!("person-stop-{browser}"), profile).await;
    let mut events = core.subscribe(None);
    let calls = runtime(&core);
    for state in ["stopping", "stopped", "stopped"] {
        status(&core, state, Instant::now());
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while core.shared.store.view("a").unwrap().state["turns"][0]["status"] != "interrupted" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("browser Stop must reach Vibe and project its Interrupted turn");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![json!({"sessionId":"a","expectedTurnId":"one"})]
    );
    assert_eq!(crate::stops::generation(&core.shared, "b"), 0);
    assert_eq!(
        core.shared.store.view("b").unwrap().state["turns"][0]["status"],
        "in_progress"
    );
    let notice = tokio::time::timeout(Duration::from_secs(1), async {
        let mut notice = None;
        let mut returned = false;
        loop {
            let event = events.next().await.unwrap();
            let value = serde_json::to_value(event).unwrap();
            if value["change"]["kind"] == "notice" {
                notice = Some(value.clone());
            }
            if value["type"] == "queueReturned" {
                assert_eq!(value["sessionId"], "a");
                assert_eq!(value["items"][0]["id"], "queued");
                returned = true;
            }
            if returned && let Some(notice) = notice.take() {
                break notice;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(notice["change"]["sessionId"], "a");
    assert_eq!(
        notice["change"]["params"]["warning"]["message"],
        format!("Stopped in Your {browser} by you")
    );
    for action in ["get", "cancel", "stop"] {
        let reason = gateway::call(
            &core.shared,
            "a",
            "chrome_session",
            json!({"action":action,"sessionId":"task-a"}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            reason,
            super::person_stop::refusal(Browser::from_profile(profile))
        );
    }
    turn(&core, "a", "two", true);
    for args in [
        json!({"sessionId":"task-a","action":"page.read","args":{}}),
        json!({"sessionId":"task-a","steps":[{"action":"page.read","args":{}}]}),
    ] {
        let reason = gateway::call(&core.shared, "a", "chrome_execute", args)
            .await
            .unwrap_err();
        assert_eq!(
            reason,
            format!(
                "Not run: this {browser} session was stopped; open a new one with chrome_session."
            )
        );
    }
    let opened = gateway::call(
        &core.shared,
        "a",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["sessionId"],
        "new-task"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_person_stop_interrupts_only_its_chat_once() {
    interrupted_once("chrome:p", "Chrome").await;
}

#[cfg(feature = "your-edge")]
#[tokio::test]
async fn edge_person_stop_interrupts_only_its_chat_once() {
    interrupted_once("edge:p", "Edge").await;
}

#[tokio::test]
async fn refusal_names_your_stop_and_ends_browser_calls() {
    // Edge only with its feature, as the Edge test above: without it the gateway refuses Edge as not in this
    // version before any Stop check (`cargo test -p vw-core-api` alone builds no `your-edge`).
    let profiles: &[&str] = if cfg!(feature = "your-edge") {
        &["chrome:p", "edge:p"]
    } else {
        &["chrome:p"]
    };
    for &profile in profiles {
        let browser = Browser::from_profile(profile);
        let core = fixture(&format!("person-stop-refusal-{profile}"), profile).await;
        status(&core, "stopping", Instant::now());
        for (tool, args) in [
            (
                "chrome_session",
                json!({"action":"open","profileId":profile}),
            ),
            (
                "chrome_execute",
                json!({"sessionId":"task-a","action":"page.read","args":{}}),
            ),
        ] {
            assert_eq!(
                gateway::call(&core.shared, "a", tool, args)
                    .await
                    .unwrap_err(),
                format!(
                    "Not run: the user pressed Stop in {}. Do not use the browser again in this turn; tell them what you were doing and wait for their answer.",
                    browser.name()
                )
            );
        }
        core.shutdown().await;
    }
}

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
