//! Atomic page adoption controls: stale requests, reused IDs, overlap, live tail and projected-page bounds.
//! Uses only `SessionStore`; no transport, timer or window.
use crate::{Delivery, HISTORY_LIMIT, LoadKind, Notification, SessionStore};
use serde_json::{Value, json};

fn initial() -> Value {
    json!({"session":{"id":"s"}, "eventId":0,"history":[{"id":"new","message":"current"}],"turns":[],"historyBeforeCursor":"older"})
}

#[test]
fn replacements_and_remove_recreate_reject_even_an_identical_cursor() {
    for kind in [
        LoadKind::New,
        LoadKind::Read,
        LoadKind::Resume,
        LoadKind::Replace,
    ] {
        let store = SessionStore::new();
        let _ = store.load(initial(), LoadKind::New).unwrap();
        let request = store.history_request("s").unwrap();
        let _ = store.load(initial(), kind).unwrap();
        assert!(!store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
        assert_eq!(store.view("s").unwrap().state, initial());
    }
    let store = SessionStore::new();
    let _ = store.load(initial(), LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    let _ = store.remove("s");
    let _ = store.load(initial(), LoadKind::New).unwrap();
    assert!(!store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
}

#[test]
fn closing_or_receiving_a_snapshot_invalidates_pending_pages() {
    let store = SessionStore::new();
    let _ = store.load(initial(), LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    store.close_history("s");
    assert!(store.history_request("s").is_none());
    assert!(!store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
    let _ = store.load(initial(), LoadKind::Resume).unwrap();
    let request = store.history_request("s").unwrap();
    let mut snapshot = initial();
    snapshot["eventId"] = json!(1);
    let _ = store.apply(Notification::new(
        "session/snapshot",
        json!({"sessionId":"s","eventId":1,"state":snapshot}),
    ));
    assert!(!store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
    assert_eq!(store.view("s").unwrap().state, snapshot);
}

#[test]
fn pages_deduplicate_without_overwriting_live_entries_or_metadata() {
    let store = SessionStore::new();
    let _ = store.load(initial(), LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    let live = json!({"id":"live","type":"message"});
    let version = store.view("s").unwrap().history_version;
    let _ = store.apply(Notification::new(
        "history/entryAdded",
        json!({"sessionId":"s","eventId":1,"entry":live}),
    ));
    let sub = store.subscribe(4);
    let _ = sub.drain();
    assert!(store.prepend_history(
        "s",
        &request,
        vec![
            json!({"id":"old"}),
            json!({"id":"old"}),
            json!({"id":"new","message":"stale"})
        ],
        None
    ));
    let view = store.view("s").unwrap();
    assert!(view.history_version > version);
    assert_eq!(
        view.state["history"],
        json!([{"id":"old"},{"id":"new","message":"current"},live])
    );
    assert_eq!(view.last_event_id, 1);
    assert_eq!(view.state["turns"], json!([]));
    assert!(matches!(sub.drain().as_slice(), [Delivery::Snapshot(v)] if **v == view));
    assert!(store.history_request("s").is_none());
    assert!(!store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
}

#[test]
fn projected_oversize_pages_keep_the_nearest_prefix_and_all_live_entries() {
    let store = SessionStore::new();
    let mut state = initial();
    state["history"] = json!(
        (1..HISTORY_LIMIT)
            .map(|i| json!({"id":format!("e{i}")}))
            .collect::<Vec<_>>()
    );
    let _ = store.load(state, LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    assert_eq!(request.limit, 1);
    assert!(store.prepend_history(
        "s",
        &request,
        vec![json!({"id":"far"}), json!({"id":"near"})],
        None
    ));
    let view = store.view("s").unwrap();
    assert_eq!(
        view.state["history"].as_array().unwrap().len(),
        HISTORY_LIMIT
    );
    assert_eq!(view.state["history"][0]["id"], "near");
    assert_eq!(view.state["historyTruncated"], true);
    assert!(store.history_request("s").is_none());
    let _ = store.apply(Notification::new(
        "history/entryAdded",
        json!({"sessionId":"s","eventId":1,"entry":{"id":"live"}}),
    ));
    assert_eq!(
        store.view("s").unwrap().state["history"]
            .as_array()
            .unwrap()
            .len(),
        HISTORY_LIMIT + 1
    );
    let _ = store.load(initial(), LoadKind::Replace).unwrap();
    assert_ne!(store.view("s").unwrap().state["historyTruncated"], true);
    assert!(store.history_request("s").is_some());
}

#[test]
fn byte_budget_keeps_a_contiguous_nearest_prefix_and_counts_the_complete_json() {
    const BUDGET: usize = 64 * 1024 * 1024;
    let store = SessionStore::new();
    let near = json!({"id":"near","unmodeled":{"text":"\"\n\\é","values":[true, null, 42]}});
    let mut state = initial();
    state["history"][0]["message"] = json!("");
    let padding = BUDGET
        - serde_json::to_vec(&state["history"]).unwrap().len()
        - serde_json::to_vec(&near).unwrap().len()
        - 1;
    state["history"][0]["message"] = Value::String("x".repeat(padding));
    let _ = store.load(state, LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    assert!(store.prepend_history(
        "s",
        &request,
        vec![
            json!({"id":"far"}),
            json!({"id":"gap","message":"too large"}),
            near.clone(),
            json!({"id":"new","message":"stale duplicate"}),
        ],
        None
    ));
    store.with(|inner| {
        let entry = &inner.sessions["s"];
        let history = &entry.state["history"];
        assert_eq!(
            history.as_array().unwrap().len(),
            2,
            "never skip a gap to keep farther entries"
        );
        assert_eq!(history[0], near);
        assert_eq!(serde_json::to_vec(history).unwrap().len(), BUDGET);
        assert!(
            entry.history_paging.truncated,
            "omissions on the last page also show the quiet line"
        );
    });
    assert!(store.history_request("s").is_none());
}

#[test]
fn live_changes_during_a_read_use_the_remaining_budget_without_being_dropped() {
    const BUDGET: usize = 64 * 1024 * 1024;
    let store = SessionStore::new();
    let _ = store.load(initial(), LoadKind::New).unwrap();
    let request = store.history_request("s").unwrap();
    // Updates and new events can put the authoritative tail over budget while a page is in flight.
    let _ = store.apply(Notification::new(
        "history/entryUpdated",
        json!({
            "sessionId":"s","eventId":1,"entryId":"new",
            "patch":[{"op":"replace","path":"/message","value":"x".repeat(BUDGET)}]
        }),
    ));
    let _ = store.apply(Notification::new(
        "history/entryAdded",
        json!({
            "sessionId":"s","eventId":2,"entry":{"id":"live"}
        }),
    ));
    assert!(store.prepend_history("s", &request, vec![json!({"id":"old"})], None));
    store.with(|inner| {
        let entry = &inner.sessions["s"];
        let history = entry.state["history"].as_array().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["message"].as_str().unwrap().len(), BUDGET);
        assert_eq!(history[1]["id"], "live");
        assert_eq!(entry.last_event_id, 2);
        assert!(entry.history_paging.truncated);
    });
    assert!(store.history_request("s").is_none());
}

#[test]
fn a_large_initial_tail_stops_background_reads_and_publishes_the_quiet_state_once() {
    let store = SessionStore::new();
    let mut state = initial();
    state["history"][0]["message"] = Value::String("x".repeat(64 * 1024 * 1024));
    let _ = store.load(state, LoadKind::New).unwrap();
    let sub = store.subscribe(4);
    let _ = sub.drain();
    assert!(store.history_request("s").is_none());
    assert!(store.history_request("s").is_none());
    assert!(
        matches!(sub.drain().as_slice(), [Delivery::Snapshot(v)] if v.state["historyTruncated"] == true)
    );
    // A history which was already complete needs no truncation notice, even with a large live tail.
    store.with(|inner| {
        inner.sessions.get_mut("s").unwrap().state["historyBeforeCursor"] = Value::Null;
        inner
            .sessions
            .get_mut("s")
            .unwrap()
            .history_paging
            .truncated = false;
    });
    assert!(store.history_request("s").is_none());
    assert_eq!(sub.drain(), Vec::<Delivery>::new());
}
