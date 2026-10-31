//! The store effect of an accepted `turn/start` on a core without Vibe: the chat reads running from Vibe's reply on,
//! before Vibe's own events for the turn arrive (they can lag the reply by tens of seconds).
use serde_json::{Value, json};
use vw_session::LoadKind;

fn idle() -> Value {
    json!({"session":{"id":"s", "harness":"unified", "createdAt":1000, "bumpedAt":2000,
        "status":{"type":"idle"}}, "eventId":3, "turns":[{"id":"t1", "status":"completed"}], "history":[]})
}

/// Vibe 3.0.3's `turn/start` reply (golden capture).
fn reply(turn: &str) -> Value {
    json!({"lastEventId":3, "turn":{"id":turn, "sessionId":"s", "status":"in_progress", "inputEntryId":null,
        "startedAt":5, "completedAt":null, "error":null, "stopReason":null, "queueItemId":null}})
}

/// What the window's composer reads to decide between a new turn and steer or queue
/// (`ui/src/features/composer/model/session.ts` `runningTurnId`).
fn running_turn(state: &Value) -> Option<String> {
    let status = &state["session"]["status"];
    if matches!(status["type"].as_str(), Some("running" | "blocked")) {
        return status["activeTurnId"].as_str().map(str::to_owned);
    }
    state["turns"]
        .as_array()?
        .iter()
        .rev()
        .find(|t| t["status"] == "in_progress")
        .and_then(|t| t["id"].as_str().map(str::to_owned))
}

#[tokio::test]
async fn an_accepted_turn_start_makes_the_chat_read_running_before_vibes_events() {
    let core = crate::offline_core::start("accepted-turn").await;
    let shared = &core.shared;
    let _ = shared.store.load(idle(), LoadKind::New).unwrap();
    for (turn, params) in [
        (
            "t2",
            json!({"sessionId":"s", "input":[{"type":"text","text":"hi"}]}),
        ),
        // The composer's retry is an injected turn: the same reply, the same rule.
        (
            "t3",
            json!({"sessionId":"s", "input":[{"type":"text","text":"continue"}], "injected":true}),
        ),
    ] {
        super::observe(shared, "turn/start", &params, &reply(turn))
            .await
            .unwrap();
        let state = super::snapshot(shared, "s").unwrap().state;
        assert_eq!(
            running_turn(&state).as_deref(),
            Some(turn),
            "the window must see the chat running {turn}: {state}"
        );
        assert!(
            state["turns"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == turn && t["status"] == "in_progress"),
            "{state}"
        );
        // Vibe ends the turn; the next accepted start is listed again.
        let mut done = reply(turn)["turn"].clone();
        done["status"] = json!("completed");
        let view = shared.store.view("s").unwrap();
        let next = view.last_event_id + 1;
        let _ = shared.store.apply(vw_session::Notification::new(
            "turn/completed",
            json!({"sessionId":"s", "eventId":next, "turn":done}),
        ));
        let _ = shared.store.apply(vw_session::Notification::new(
            "session/updated",
            json!({"sessionId":"s", "eventId":next + 1, "patch":[
                {"op":"remove","path":"/status/activeTurnId"},
                {"op":"replace","path":"/status/type","value":"idle"}]}),
        ));
        let state = super::snapshot(shared, "s").unwrap().state;
        assert_eq!(running_turn(&state), None, "{state}");
    }
}

/// `session/turn/enqueue` answers only a queue item id: nothing is listed (Vibe's queue events say what runs).
#[tokio::test]
async fn an_enqueue_reply_lists_no_turn() {
    let core = crate::offline_core::start("accepted-enqueue").await;
    let shared = &core.shared;
    let _ = shared.store.load(idle(), LoadKind::New).unwrap();
    super::observe(
        shared,
        "session/turn/enqueue",
        &json!({"sessionId":"s", "entries":[]}),
        &json!({"queueItemId":"q1"}),
    )
    .await
    .unwrap();
    let state = super::snapshot(shared, "s").unwrap().state;
    assert_eq!(running_turn(&state), None);
    assert_eq!(state["turns"].as_array().unwrap().len(), 1);
}
