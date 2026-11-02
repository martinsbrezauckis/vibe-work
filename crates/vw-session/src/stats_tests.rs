//! Stats retention tests for the session reducer, using synthetic notifications and authoritative rereads.
use crate::{Change, LoadKind, Notification, SessionStore};
use serde_json::json;

#[test]
fn stats_keep_prices_steps_and_tool_counts_through_a_reread() {
    let store = SessionStore::new();
    let initial = json!({"session":{"id":"s","status":{"type":"idle"}},"history":[],"turns":[],"childSessions":[],"eventId":0});
    let _ = store.load(initial.clone(), LoadKind::New).unwrap();
    let stats = json!({"sessionPromptTokens":1000,"sessionCompletionTokens":100,
        "sessionCachedTokens":800,"inputPricePerMillion":2,"outputPricePerMillion":6,
        "cachedInputPricePerMillion":0.2,"steps":3,"toolCallsSucceeded":2,"toolCallsFailed":1});
    let applied = store.apply(Notification {
        method: "session/statsUpdated".into(),
        params: json!({"sessionId":"s","eventId":1,"stats":stats}),
    });
    assert_eq!(store.view("s").unwrap().state["session"]["stats"], stats);
    assert!(applied.changes.iter().any(|change| matches!(change, Change::SessionPatched { patch, .. } if patch.iter().any(|op| op["path"] == "/stats"))));
    let mut read = initial;
    read["eventId"] = json!(1);
    let _ = store.load(read, LoadKind::Read).unwrap();
    assert_eq!(store.view("s").unwrap().state["session"]["stats"], stats);
}
