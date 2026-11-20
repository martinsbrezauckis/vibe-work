//! The agent control frame and the person's Pause on helper browsers, through the owner's observer and the gateway
//! with a fake helper; isolated cores, no browser or desktop.
//!
//! PREDICT (written before the frame and pause wiring): no `control.frame` is ever sent (the helper sees only the
//! agent's own calls); a helper row with `personPause` changes nothing, so the chat is never paused; a paused chat's
//! `chrome_execute` reaches the helper at once instead of waiting; the first result after Resume carries no note.
use crate::chrome_control::{gateway, owner, test_support};
use crate::paused_tools::{BROWSER, CHROME_RESUMED_NOTE};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

async fn fixture(name: &str) -> (crate::Core, Seen) {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        // The chats' records folder, where a chat's frame colour is kept.
        config.sessions_dir = Some(dir.join("sessions"));
    })
    .await;
    turn(&core, "a", "one", true);
    core.shared.access.observe_request(
        "session/agent/update",
        "a",
        &json!({"agentName":"auto-approve"}),
    );
    core.shared.chrome_control.work.lock().unwrap().open(
        "task-a",
        "a",
        "active",
        "chrome:p",
        Some("one"),
    );
    core.shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("task-a".into(), "a".into());
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_execute" {
            json!({"content":[{"type":"text","text":"page text"}],
                   "structuredContent":{"sessionId":args["sessionId"],"status":"completed"}})
        } else if tool == "chrome_session" && args["action"] == "frame" {
            json!({"structuredContent":{"sessionId":args["sessionId"],"state":"active",
                   "frame":{"state":args["state"],"label":args["label"],"colour":args["colour"]}}})
        } else {
            json!({"structuredContent":{"sessionId":args["sessionId"],"state":"active"}})
        }
    });
    (core, seen)
}

fn turn(core: &crate::Core, chat: &str, id: &str, running: bool) {
    let _ = core.shared.store.load(json!({
        "session":{"id":chat,"status":{"type":if running {"running"} else {"idle"},"activeTurnId":running.then_some(id)}},
        "history":[],"turns":[{"id":id,"status":if running {"in_progress"} else {"completed"}}]
    }), vw_session::LoadKind::New).unwrap();
}

fn rows(core: &crate::Core, rows: &Value) {
    owner::update_work(
        &Arc::downgrade(&core.shared),
        &json!({"structuredContent":{"sessions":rows}}),
        Instant::now(),
    );
}

async fn frames(seen: &Seen) -> Vec<Value> {
    // `control` is fire-and-forget: let the fake helper task take the queued calls.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(tool, args)| tool == "chrome_session" && args["action"] == "frame")
        .map(|(_, args)| args.clone())
        .collect()
}

/// PREDICT (before detached control calls): the owner's queue drops a call whose caller stopped waiting unless it is
/// a stop, delete or cancel, and `control` never waits, so the frame update is dropped before the helper; the
/// helper row then has no `frame` and the extension draws only its older corner marks. The fake helper applies the
/// real owner's admission rule (`owner::Call::admitted`).
#[tokio::test]
async fn a_frame_update_passes_the_owners_queue_with_the_fields_the_helper_requires() {
    let (core, seen) = fixture("frame-admission").await;
    crate::chrome_control::frames::sync_chat(&core.shared, "a");
    let sent = frames(&seen).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    // The helper's `control.frame` requires exactly these fields (plus the tool's `action`).
    let mut keys: Vec<&str> = sent[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "ackPause",
            "action",
            "colour",
            "label",
            "sessionId",
            "state"
        ]
    );
    assert_eq!(sent[0]["sessionId"], "task-a");
    assert_eq!(sent[0]["state"], "working");
}

#[tokio::test]
async fn working_frame_once_then_off_at_turn_end_with_the_session_kept() {
    let (core, seen) = fixture("frame-working-off").await;
    super::sync_chat(&core.shared, "a");
    super::sync_chat(&core.shared, "a");
    let sent = frames(&seen).await;
    assert_eq!(sent.len(), 1, "only a change is sent: {sent:?}");
    assert_eq!(sent[0]["state"], "working");
    assert_eq!(sent[0]["sessionId"], "task-a");
    assert_eq!(sent[0]["colour"], 0);
    assert_eq!(sent[0]["ackPause"], 0);
    assert!(sent[0]["label"].as_str().unwrap().chars().count() <= 40);
    turn(&core, "a", "one", false);
    crate::chrome_control::end_turn(&core.shared, "a");
    let sent = frames(&seen).await;
    assert_eq!(sent.last().unwrap()["state"], "off");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| matches!(args["action"].as_str(), Some("stop" | "delete"))),
        "the session stays for a follow-up"
    );
    core.shutdown().await;
}

/// PREDICT: after an idle turn has switched the frame off, the first awaited browser action
/// enters the helper before the next working frame is queued. The person has no visible Stop
/// until that action finishes, even though the chat turn is running.
#[tokio::test]
async fn first_script_of_a_followup_turn_queues_working_frame_before_action() {
    let (core, seen) = fixture("frame-before-first-script").await;
    super::sync_chat(&core.shared, "a");
    turn(&core, "a", "one", false);
    crate::chrome_control::end_turn(&core.shared, "a");
    assert_eq!(frames(&seen).await.last().unwrap()["state"], "off");
    turn(&core, "a", "two", true);
    let result = gateway::call(
        &core.shared,
        "a",
        "chrome_execute",
        json!({"sessionId":"task-a","action":"raw.execute","args":{"expression":"new Promise(()=>{})"}}),
    ).await;
    let calls = seen.lock().unwrap().clone();
    let tail: Vec<_> = calls
        .iter()
        .filter(|(name, args)| {
            name == "chrome_execute" && args["action"] == "raw.execute"
                || name == "chrome_session"
                    && args["action"] == "frame"
                    && args["state"] == "working"
        })
        .collect();
    assert!(
        result.is_ok(),
        "first script did not reach the fake helper: {result:?}"
    );
    assert_eq!(
        tail.len(),
        3,
        "expected old working, new working, and raw action: {calls:?}"
    );
    assert_eq!(
        tail[1].0, "chrome_session",
        "working frame must precede the script: {calls:?}"
    );
    assert_eq!(tail[1].1["state"], "working");
    assert_eq!(tail[2].0, "chrome_execute", "{calls:?}");
    core.shutdown().await;
}

/// PREDICT: a rejected or mismatched frame answer lets a raw script reach the helper while the
/// person still lacks confirmed controls. The action must end before dispatch in either case.
#[tokio::test]
async fn first_script_refuses_when_its_frame_cannot_be_confirmed() {
    for frame_state in ["wrong", "rejected"] {
        let (core, _) = fixture(&format!("frame-unconfirmed-{frame_state}")).await;
        let seen = test_support::answering(&core.shared, move |tool, args| {
            if tool == "chrome_session" && args["action"] == "frame" {
                if frame_state == "rejected" {
                    json!({"isError":true,"structuredContent":{"error":{"code":"CONTROL_STATE"}}})
                } else {
                    json!({"structuredContent":{"sessionId":args["sessionId"],"frame":{"state":"off"}}})
                }
            } else {
                json!({"structuredContent":{"sessionId":args["sessionId"],"status":"completed","state":"active"}})
            }
        });
        let result = gateway::call(
            &core.shared,
            "a",
            "chrome_execute",
            json!({"sessionId":"task-a","action":"raw.execute","args":{"expression":"new Promise(()=>{})"}}),
        ).await;
        assert!(
            result.unwrap_err().starts_with("Not run:"),
            "unconfirmed controls must refuse {frame_state}"
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, _)| tool == "chrome_execute"),
            "script reached the helper after {frame_state} frame receipt"
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_person_pause_row_pauses_once_and_is_acknowledged() {
    let (core, seen) = fixture("frame-person-pause").await;
    super::sync_chat(&core.shared, "a");
    let pause =
        json!([{"sessionId":"task-a","state":"active","personPause":{"state":"paused","seq":1}}]);
    rows(&core, &pause);
    assert!(core.shared.paused_tools.is_paused("a", BROWSER));
    assert_eq!(core.shared.paused_tools.view("a").paused[0].place, "Chrome");
    let sent = frames(&seen).await;
    let last = sent.last().unwrap();
    assert_eq!(
        (last["state"].as_str(), last["ackPause"].as_u64()),
        (Some("paused"), Some(1))
    );
    // The same row again (every poll) changes nothing; the app's Resume is not undone by it.
    core.shared.paused_tools.resume("a", None);
    rows(&core, &pause);
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    rows(
        &core,
        &json!([{"sessionId":"task-a","state":"active","personPause":{"state":"paused","seq":2}}]),
    );
    assert!(core.shared.paused_tools.is_paused("a", BROWSER));
    rows(
        &core,
        &json!([{"sessionId":"task-a","state":"active","personPause":{"state":"resumed","seq":3}}]),
    );
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    let sent = frames(&seen).await;
    let last = sent.last().unwrap();
    assert_eq!(
        (last["state"].as_str(), last["ackPause"].as_u64()),
        (Some("working"), Some(3))
    );
    core.shutdown().await;
}

// PREDICT before the fix: a helper frameControl notification is ignored until a status/action reply arrives.
#[tokio::test]
async fn signed_frame_notification_applies_one_chat_immediately_and_ignores_stale_or_malformed_rows()
 {
    let (core, _) = fixture("frame-control-notification").await;
    turn(&core, "b", "two", true);
    core.shared.chrome_control.work.lock().unwrap().open(
        "task-b",
        "b",
        "active",
        "chrome:q",
        Some("two"),
    );
    let event = |session: &str, chat: &str, profile: &str, state: &str, seq: Value| {
        json!({
            "method":"notifications/owner/frameControl",
            "params":{"sessionId":session,"clientSessionId":chat,"profileId":profile,"state":"active",
                      "personPause":{"state":state,"seq":seq}}
        })
    };
    let owner = Arc::downgrade(&core.shared);
    let pause = event("task-a", "a", "chrome:p", "paused", json!(1));
    owner::observe(&owner, &pause).unwrap();
    assert!(
        core.shared.paused_tools.is_paused("a", BROWSER),
        "Pause applies before any status poll"
    );
    assert!(!core.shared.paused_tools.is_paused("b", BROWSER));
    for invalid in [
        event("task-a", "b", "chrome:p", "paused", json!(2)),
        event("task-a", "a", "chrome:q", "paused", json!(2)),
        event("missing", "a", "chrome:p", "paused", json!(2)),
        event("task-a", "a", "chrome:p", "stopped", json!(2)),
        event("task-a", "a", "chrome:p", "paused", json!("two")),
    ] {
        owner::observe(&owner, &invalid).unwrap();
        assert!(
            core.shared.paused_tools.is_paused("a", BROWSER),
            "{invalid}"
        );
        assert!(
            !core.shared.paused_tools.is_paused("b", BROWSER),
            "{invalid}"
        );
    }
    owner::observe(
        &owner,
        &event("task-a", "a", "chrome:p", "resumed", json!(2)),
    )
    .unwrap();
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    owner::observe(&owner, &pause).unwrap();
    assert!(
        !core.shared.paused_tools.is_paused("a", BROWSER),
        "stale seq must not replay Pause"
    );
    owner::observe(
        &owner,
        &event("task-b", "b", "chrome:q", "paused", json!(1)),
    )
    .unwrap();
    assert!(core.shared.paused_tools.is_paused("b", BROWSER));
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shared.chrome_control.work.lock().unwrap().open(
        "task-a",
        "a",
        "stopped",
        "chrome:p",
        Some("one"),
    );
    super::observe(
        &core.shared,
        &[json!({"sessionId":"task-a","state":"active",
        "personPause":{"state":"paused","seq":3}})],
    );
    assert!(
        !core.shared.paused_tools.is_paused("a", BROWSER),
        "a stale active row must not pause current stopped work"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_pause_click_without_a_running_turn_is_switched_off() {
    let (core, seen) = fixture("frame-pause-idle").await;
    turn(&core, "a", "one", false);
    rows(
        &core,
        &json!([{"sessionId":"task-a","state":"active","personPause":{"state":"paused","seq":1}}]),
    );
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    let sent = frames(&seen).await;
    let last = sent
        .last()
        .expect("the helper's optimistic pause is answered");
    assert_eq!(
        (last["state"].as_str(), last["ackPause"].as_u64()),
        (Some("off"), Some(1))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_paused_chrome_call_waits_until_resume_and_notes_the_page_may_have_changed() {
    let (core, seen) = fixture("frame-hold-resume").await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let call = tokio::spawn({
        let shared = core.shared.clone();
        async move {
            gateway::call(
                &shared,
                "a",
                "chrome_execute",
                json!({"sessionId":"task-a","action":"page.read","args":{}}),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!call.is_finished(), "a paused call waits");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute"),
        "nothing reached the page while paused"
    );
    assert!(core.shared.paused_tools.view("a").paused[0].waiting);
    core.shared.paused_tools.resume("a", None);
    let answer = call.await.unwrap().expect("runs after Resume");
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    // PREDICT (written before the change): the note said only that the page may have changed, so an agent whose
    // network.intercept rules ended at the Pause (by design) went on as if they still applied.
    let note = answer["vwMcpResult"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert_eq!(note, CHROME_RESUMED_NOTE);
    assert!(note.contains("The page may have changed"), "{note}");
    assert!(note.contains("network.intercept"), "{note}");
    assert!(note.contains("add them again"), "{note}");
    core.shutdown().await;
}

#[tokio::test]
async fn stop_while_paused_never_runs_the_waiting_chrome_call() {
    let (core, seen) = fixture("frame-hold-stop").await;
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let call = tokio::spawn({
        let shared = core.shared.clone();
        async move {
            gateway::call(
                &shared,
                "a",
                "chrome_execute",
                json!({"sessionId":"task-a","action":"page.read","args":{}}),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    crate::stops::stopped(&core.shared, "a");
    let reason = call.await.unwrap().unwrap_err();
    assert!(reason.starts_with("Not run:"), "{reason}");
    core.shared.paused_tools.resume("a", None);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute"),
        "a stopped call never runs later"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_ended_session_row_never_pauses() {
    let (core, _seen) = fixture("frame-ended-row").await;
    rows(
        &core,
        &json!([{"sessionId":"task-a","state":"stopped","personPause":{"state":"paused","seq":9}}]),
    );
    assert!(!core.shared.paused_tools.is_paused("a", BROWSER));
    core.shutdown().await;
}

/// PREDICT (written before the setting): the colour lives in an in-memory map nobody can set, so every frame is
/// index 0 and a restart forgets any colour.
#[tokio::test]
async fn a_set_chat_colour_reaches_the_helper_frame_and_survives_a_reload() {
    let (core, seen) = fixture("frame-colour-kept").await;
    super::sync_chat(&core.shared, "a");
    assert_eq!(
        frames(&seen).await[0]["colour"],
        0,
        "one colour for every chat by default"
    );
    assert!(super::set_colour(&core.shared, "a", super::super::frame_colours::PALETTE).is_err());
    super::set_colour(&core.shared, "a", 3).unwrap();
    let sent = frames(&seen).await;
    assert_eq!(sent.last().unwrap()["colour"], 3, "{sent:?}");
    assert_eq!(super::colours(&core.shared).get("a"), Some(&3));
    let mut reloaded = super::Frames::default();
    let dir = core.shared.config.sessions_dir.as_deref();
    assert!(dir.is_some());
    assert_eq!(reloaded.colour(dir, "a"), 3, "kept with the chat's records");
    assert_eq!(reloaded.colour(dir, "b"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn the_frame_label_is_guarded_like_a_title() {
    let f = crate::feature_test_core::Fixture::new().await;
    let shared = &f.core.shared;
    crate::title_guard::rename_tests::know(shared, crate::title_guard::rename_tests::VALUE);
    shared
        .names
        .note_listing(&[json!({"id":"a", "title":"zebrafish3c9e71a4 check"})]);
    assert_eq!(super::label(shared, "a"), "[secret withheld] check");
    f.finish().await;
}
