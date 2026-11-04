//! Resume history regressions on synthetic long chats. Uses the store and its subscriber API;
//! the crate test runner checks that a bounded reopen preserves only a verified loaded prefix.

use serde_json::{Value, json};

use crate::{Change, Delivery, LoadKind, Notification, SessionStore};

fn message(id: &str, role: &str) -> Value {
    json!({"id":id, "type":"message", "role":role,
        "content":[{"type":"text", "text":id}]})
}

fn loaded() -> Value {
    let mut history = vec![
        message("early-prompt", "user"),
        message("early-reply", "assistant"),
    ];
    history.extend((0..220).map(|i| {
        if i % 2 == 0 {
            json!({"id":format!("step-{i}"), "type":"effect", "state":{"outputText":"browser result"}})
        } else {
            json!({"id":format!("step-{i}"), "type":"notice", "text":"browser step finished"})
        }
    }));
    history.extend([
        message("recent-prompt", "user"),
        message("recent-reply", "assistant"),
    ]);
    json!({"session":{"id":"s", "status":{"type":"idle"}}, "eventId":100,
        "history":history, "historyBeforeCursor":"before-loaded",
        "turns":[{"id":"early-turn", "status":"completed"},
            {"id":"recent-turn", "status":"completed"}], "activeCallbacks":[]})
}

fn tail(full: &Value) -> Value {
    let mut page = full.clone();
    let history = full["history"].as_array().unwrap();
    page["history"] = json!(&history[history.len() - 200..]);
    page["historyBeforeCursor"] = json!("before-tail");
    page["turns"] = json!([{"id":"recent-turn", "status":"completed"}]);
    page
}

fn resumed() -> (SessionStore, crate::Subscription, Value) {
    let store = SessionStore::new();
    let full = loaded();
    let _ = store.load(full.clone(), LoadKind::New).unwrap();
    let sub = store.subscribe(8);
    let _ = sub.drain();
    // Closing a channel leaves the store loaded. Reopening supplies only Vibe's newest page.
    let _ = store.load(tail(&full), LoadKind::Resume).unwrap();
    (store, sub, full)
}

fn assert_markers(state: &Value) {
    let history = state["history"].as_array().unwrap();
    for id in [
        "early-prompt",
        "early-reply",
        "recent-prompt",
        "recent-reply",
    ] {
        assert!(
            history.iter().any(|entry| entry["id"] == id),
            "missing {id}"
        );
    }
}

#[test]
fn same_session_resume_keeps_loaded_history_in_store_snapshot() {
    let (store, _, full) = resumed();
    let view = store.view("s").unwrap();
    assert_markers(&view.state);
    assert_eq!(view.state["history"], full["history"]);
    assert_eq!(view.state["historyBeforeCursor"], "before-loaded");
}

#[test]
fn same_session_resume_keeps_loaded_history_in_subscriber_replacement() {
    let (_, sub, full) = resumed();
    let deliveries = sub.drain();
    let [Delivery::Snapshot(view)] = deliveries.as_slice() else {
        panic!("expected the resume replacement, got {deliveries:?}");
    };
    assert_markers(&view.state);
    assert_eq!(view.state["history"], full["history"]);
    assert_eq!(view.state["historyBeforeCursor"], "before-loaded");
}

#[test]
fn resumed_page_entries_win_and_a_new_turn_appends_normally() {
    let store = SessionStore::new();
    let mut full = loaded();
    full["historyBeforeCursor"] = Value::Null;
    let _ = store.load(full.clone(), LoadKind::New).unwrap();
    let mut page = tail(&full);
    page["history"][199]["content"] = json!([{"type":"text", "text":"updated reply"}]);
    let _ = store.load(page.clone(), LoadKind::Resume).unwrap();
    let sub = store.subscribe(8);
    let _ = sub.drain();
    for (event, method, params) in [
        (
            101,
            "turn/started",
            json!({"turn":{"id":"next-turn", "status":"in_progress"}}),
        ),
        (
            102,
            "history/entryAdded",
            json!({"entry":message("next-prompt", "user")}),
        ),
        (
            103,
            "history/entryAdded",
            json!({"entry":message("next-reply", "assistant")}),
        ),
        (
            104,
            "turn/completed",
            json!({"turn":{"id":"next-turn", "status":"completed"}}),
        ),
    ] {
        let mut params = params;
        params["sessionId"] = json!("s");
        params["eventId"] = json!(event);
        let applied = store.apply(Notification::new(method, params));
        assert!(applied.actions.is_empty(), "{applied:?}");
    }
    let view = store.view("s").unwrap();
    assert_markers(&view.state);
    let history = view.state["history"].as_array().unwrap();
    assert_eq!(history.len(), 226);
    assert_eq!(history[223], page["history"][199]);
    assert_eq!(history[224], message("next-prompt", "user"));
    assert_eq!(history[225], message("next-reply", "assistant"));
    assert_eq!(view.last_event_id, 104);
    assert!(!view.resync_pending);
    assert_eq!(view.state["historyBeforeCursor"], Value::Null);
    let deliveries = sub.drain();
    assert_eq!(
        deliveries
            .iter()
            .filter(|d| matches!(
                d,
                Delivery::Change(Change::EntryUpserted {
                    replaced: false,
                    ..
                })
            ))
            .count(),
        2
    );
}

#[test]
fn resume_without_consistent_overlap_adopts_only_the_page() {
    let full = loaded();
    let mut reversed = tail(&full);
    reversed["history"].as_array_mut().unwrap().swap(1, 2);
    let mut incomplete = tail(&full);
    incomplete["history"].as_array_mut().unwrap().pop();
    let mut disjoint = tail(&full);
    disjoint["history"] = json!([message("unknown", "user")]);
    let mut empty = tail(&full);
    empty["history"] = json!([]);
    for page in [reversed, incomplete, disjoint, empty] {
        let store = SessionStore::new();
        let _ = store.load(full.clone(), LoadKind::New).unwrap();
        let _ = store.load(page.clone(), LoadKind::Resume).unwrap();
        let view = store.view("s").unwrap();
        assert_eq!(view.state["history"], page["history"]);
        assert_eq!(view.state["historyBeforeCursor"], "before-tail");
    }
}

#[test]
fn replacements_and_new_sessions_do_not_recover_a_removed_prefix() {
    let full = loaded();
    for kind in [LoadKind::Replace, LoadKind::New] {
        for history in [json!([]), tail(&full)["history"].clone()] {
            let store = SessionStore::new();
            let _ = store.load(full.clone(), LoadKind::New).unwrap();
            let mut page = tail(&full);
            page["history"] = history.clone();
            let _ = store.load(page.clone(), kind).unwrap();
            assert_eq!(store.view("s").unwrap().state["history"], history);
            // A later resume must not restore history removed by clear/rewind.
            let _ = store.load(page, LoadKind::Resume).unwrap();
            assert_eq!(store.view("s").unwrap().state["history"], history);
        }
    }
    let store = SessionStore::new();
    let _ = store.load(full.clone(), LoadKind::New).unwrap();
    let mut imported = tail(&full);
    imported["session"]["id"] = json!("imported");
    let _ = store.load(imported.clone(), LoadKind::Resume).unwrap();
    assert_eq!(
        store.view("imported").unwrap().state["history"],
        imported["history"]
    );
    assert_eq!(store.view("s").unwrap().state, full);
}

#[test]
fn merged_resume_still_interrupts_missing_running_turns() {
    let store = SessionStore::new();
    let mut full = loaded();
    // A turn-prefix merge would keep this running turn and hide its absence from the resume.
    full["turns"][0]["status"] = json!("in_progress");
    let _ = store.load(full.clone(), LoadKind::New).unwrap();
    let sub = store.subscribe(8);
    let _ = sub.drain();
    let _ = store.load(tail(&full), LoadKind::Resume).unwrap();
    let view = store.view("s").unwrap();
    assert_markers(&view.state);
    assert_eq!(view.interrupted_turns, ["early-turn"]);
    assert_eq!(view.anomalies.interrupted_turns, 1);
    assert_eq!(
        view.state["turns"],
        json!([
            {"id":"recent-turn", "status":"completed"},
            {"id":"early-turn", "status":"interrupted"}
        ])
    );
    assert!(sub.drain().iter().any(|d| matches!(d,
        Delivery::Change(Change::TurnUpserted { turn, .. })
            if turn["id"] == "early-turn" && turn["status"] == "interrupted")));
}

#[test]
fn merged_resume_with_lower_watermark_discards_the_old_event_log() {
    let store = SessionStore::new();
    let full = loaded();
    let _ = store.load(full.clone(), LoadKind::New).unwrap();
    let patch = |event, title| {
        Notification::new(
            "session/updated",
            json!({"sessionId":"s", "eventId":event,
            "patch":[{"op":"add", "path":"/title", "value":title}]}),
        )
    };
    let _ = store.apply(patch(101, "old epoch"));
    // Hold an event past a gap too; neither it nor the applied log may replay after the reset.
    let _ = store.apply(patch(103, "stale held event"));
    assert!(store.view("s").unwrap().resync_pending);
    let sub = store.subscribe(8);
    let _ = sub.drain();
    let mut page = tail(&full);
    page["eventId"] = json!(5);
    page["session"]["title"] = json!("resumed");
    let applied = store.load(page, LoadKind::Resume).unwrap();
    assert!(applied.actions.is_empty(), "{applied:?}");
    let view = store.view("s").unwrap();
    assert_markers(&view.state);
    assert_eq!(view.last_event_id, 5);
    assert!(!view.resync_pending);
    assert_eq!(view.state["session"]["title"], "resumed");
    assert!(matches!(sub.drain().as_slice(), [Delivery::Snapshot(_)]));
    let applied = store.apply(patch(6, "new epoch"));
    assert!(applied.actions.is_empty(), "{applied:?}");
    let view = store.view("s").unwrap();
    assert_eq!(view.last_event_id, 6);
    assert_eq!(view.state["session"]["title"], "new epoch");
    assert_markers(&view.state);
}

#[test]
fn cold_resume_can_only_show_the_returned_tail() {
    let store = SessionStore::new();
    let page = tail(&loaded());
    let _ = store.load(page.clone(), LoadKind::Resume).unwrap();
    assert_eq!(store.view("s").unwrap().state, page);
}
