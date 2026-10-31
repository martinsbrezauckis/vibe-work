//! Tests of the accepted-turn rule (`SessionStore::note_accepted_turn`): a turn Vibe accepted is listed as running
//! until Vibe's own events for it arrive, which can lag the `turn/start` reply by tens of seconds.

use serde_json::{Value, json};

use crate::{Action, Change, Delivery, LoadKind, Notification, SessionStore};

fn idle_state(event: u64) -> Value {
    json!({"session":{"id":"s", "harness":"unified", "createdAt":1000, "bumpedAt":2000,
        "status":{"type":"idle"}}, "eventId":event,
        "turns":[{"id":"t1", "status":"completed"}], "history":[]})
}

/// `turn/start`'s reply turn as Vibe 3.0.3 answers it (`inputEntryId` is still null there).
fn accepted(id: &str) -> Value {
    json!({"id":id, "sessionId":"s", "status":"in_progress", "inputEntryId":null, "startedAt":5,
        "completedAt":null, "error":null, "stopReason":null, "queueItemId":null})
}

fn event(method: &str, event: u64, mut params: Value) -> Notification {
    params["sessionId"] = json!("s");
    params["eventId"] = json!(event);
    Notification::new(method, params)
}

/// Vibe 3.0.3's own events for turn `id` (golden capture order): status running, the turn started, the turn
/// completed, status idle again.
fn vibe_turn(id: &str, first: u64) -> Vec<Notification> {
    let turn =
        |status: &str| json!({"id":id, "sessionId":"s", "status":status, "inputEntryId":"m"});
    vec![
        event(
            "session/updated",
            first,
            json!({"patch":[
            {"op":"add", "path":"/status/activeTurnId", "value":id},
            {"op":"replace", "path":"/status/type", "value":"running"}]}),
        ),
        event(
            "turn/started",
            first + 1,
            json!({"turn": turn("in_progress")}),
        ),
        event(
            "turn/completed",
            first + 2,
            json!({"turn": turn("completed")}),
        ),
        event(
            "session/updated",
            first + 3,
            json!({"patch":[
            {"op":"remove", "path":"/status/activeTurnId", "value":null},
            {"op":"replace", "path":"/status/type", "value":"idle"}]}),
        ),
    ]
}

fn turn_status(store: &SessionStore, id: &str) -> Option<Value> {
    let view = store.view("s")?;
    let turns = view.state["turns"].as_array()?.clone();
    turns
        .into_iter()
        .find(|t| t["id"] == id)
        .map(|t| t["status"].clone())
}

#[test]
fn an_accepted_turn_is_listed_running_until_vibes_events_replace_it() {
    let store = SessionStore::new();
    let _ = store.load(idle_state(3), LoadKind::New).unwrap();
    let sub = store.subscribe(64);
    let _ = sub.drain();

    let applied = store.note_accepted_turn("s", &accepted("t2"));
    let view = store.view("s").unwrap();
    assert_eq!(turn_status(&store, "t2"), Some(json!("in_progress")));
    assert_eq!(view.state["session"]["status"]["type"], "running");
    assert_eq!(view.state["session"]["status"]["activeTurnId"], "t2");
    assert!(
        applied.actions.is_empty(),
        "no resync: {:?}",
        applied.actions
    );
    let published: Vec<Delivery> = sub.drain();
    assert!(
        published.iter().any(|d| matches!(d,
            Delivery::Change(Change::TurnUpserted { turn, .. }) if turn["id"] == "t2" && turn["status"] == "in_progress")),
        "the accepted turn is published: {published:?}"
    );
    assert!(
        published
            .iter()
            .any(|d| matches!(d, Delivery::Change(Change::SessionPatched { .. }))),
        "the running status is published: {published:?}"
    );

    // Vibe's real patch sequence still applies on top of the provisional status and turn.
    for n in vibe_turn("t2", 4) {
        let applied = store.apply(n);
        assert!(
            !applied
                .actions
                .iter()
                .any(|a| matches!(a, Action::NeedsResync { .. })),
            "{:?}",
            applied.actions
        );
    }
    let view = store.view("s").unwrap();
    assert!(!view.resync_pending);
    assert_eq!(view.last_event_id, 7);
    assert_eq!(view.state["session"]["status"], json!({"type":"idle"}));
    assert_eq!(view.state["turns"].as_array().unwrap().len(), 2);
    assert_eq!(turn_status(&store, "t2"), Some(json!("completed")));
    assert_eq!(view.state["turns"][1]["inputEntryId"], "m");
}

/// Vibe's events can overtake the reply: a turn Vibe already reported, finished or running, is never overwritten,
/// and the session's status is left as Vibe set it.
#[test]
fn a_turn_vibe_already_reported_is_never_overwritten() {
    let store = SessionStore::new();
    let _ = store.load(idle_state(3), LoadKind::New).unwrap();
    for n in vibe_turn("t2", 4) {
        let _ = store.apply(n);
    }
    let before = store.view("s").unwrap().state;
    let applied = store.note_accepted_turn("s", &accepted("t2"));
    assert!(applied.changes.is_empty(), "{:?}", applied.changes);
    assert_eq!(store.view("s").unwrap().state, before);

    // Running already (Vibe's status patch and turn/started arrived): nothing changes either.
    let _ = store.apply(vibe_turn("t3", 8).remove(0));
    let _ = store.apply(vibe_turn("t3", 8).remove(1));
    let before = store.view("s").unwrap().state;
    let applied = store.note_accepted_turn("s", &accepted("t3"));
    assert!(applied.changes.is_empty(), "{:?}", applied.changes);
    assert_eq!(store.view("s").unwrap().state, before);
}

/// Vibe's status patch arrived but its turn/started did not yet: the turn is listed, the status left alone.
#[test]
fn a_running_status_is_not_patched_again() {
    let store = SessionStore::new();
    let _ = store.load(idle_state(3), LoadKind::New).unwrap();
    let _ = store.apply(vibe_turn("t2", 4).remove(0));
    let applied = store.note_accepted_turn("s", &accepted("t2"));
    assert!(matches!(
        applied.changes.as_slice(),
        [Change::TurnUpserted { .. }]
    ));
    assert_eq!(turn_status(&store, "t2"), Some(json!("in_progress")));
    for n in vibe_turn("t2", 4).into_iter().skip(1) {
        let _ = store.apply(n);
    }
    let view = store.view("s").unwrap();
    assert!(!view.resync_pending);
    assert_eq!(view.state["session"]["status"], json!({"type":"idle"}));
    assert_eq!(turn_status(&store, "t2"), Some(json!("completed")));
}

/// Only a running reply turn of a loaded session counts.
#[test]
fn only_an_in_progress_turn_of_a_loaded_session_is_noted() {
    let store = SessionStore::new();
    assert_eq!(
        store.note_accepted_turn("s", &accepted("t2")).changes,
        Vec::new()
    );
    let _ = store.load(idle_state(3), LoadKind::New).unwrap();
    let mut done = accepted("t2");
    done["status"] = json!("completed");
    assert_eq!(store.note_accepted_turn("s", &done).changes, Vec::new());
    assert_eq!(
        store.note_accepted_turn("s", &json!({})).changes,
        Vec::new()
    );
    assert_eq!(turn_status(&store, "t2"), None);
    assert_eq!(
        store.view("s").unwrap().state["session"]["status"]["type"],
        "idle"
    );
}

/// A read or snapshot from Vibe replaces the provisional turn and status as usual.
#[test]
fn a_read_from_vibe_replaces_the_accepted_turn() {
    let store = SessionStore::new();
    let _ = store.load(idle_state(3), LoadKind::New).unwrap();
    let _ = store.note_accepted_turn("s", &accepted("t2"));
    let _ = store.load(idle_state(3), LoadKind::Read).unwrap();
    let view = store.view("s").unwrap();
    assert_eq!(turn_status(&store, "t2"), None);
    assert_eq!(view.state["session"]["status"]["type"], "idle");
}

/// A copy's first message: Vibe restarts its durable numbering at 1 (`bumpedAt`) after the copy setup's ephemeral
/// events. A turn accepted before that restart must not hide it (the restart rule looks for a session without turns).
#[test]
fn an_accepted_first_turn_keeps_a_copys_numbering_restart() {
    let store = SessionStore::new();
    let state = json!({"session":{"id":"s", "harness":"unified", "createdAt":1000,
        "bumpedAt":1000, "status":{"type":"idle"}}, "eventId":0, "turns":[], "history":[]});
    let _ = store.load(state, LoadKind::New).unwrap();
    let setup = event(
        "session/updated",
        1,
        json!({"patch":[{"op":"replace","path":"/cwd","value":"/copy"}]}),
    );
    let _ = store.apply(setup);
    let _ = store.note_accepted_turn("s", &accepted("t1"));
    assert_eq!(turn_status(&store, "t1"), Some(json!("in_progress")));
    let bump = event(
        "session/updated",
        1,
        json!({"patch":[{"op":"replace","path":"/bumpedAt","value":1234}]}),
    );
    let _ = store.apply(bump);
    for n in vibe_turn("t1", 2) {
        let _ = store.apply(n);
    }
    let view = store.view("s").unwrap();
    assert!(!view.resync_pending);
    assert_eq!(view.last_event_id, 5);
    assert_eq!(view.state["session"]["bumpedAt"], 1234);
    assert_eq!(turn_status(&store, "t1"), Some(json!("completed")));
    assert_eq!(view.state["session"]["status"], json!({"type":"idle"}));
}
