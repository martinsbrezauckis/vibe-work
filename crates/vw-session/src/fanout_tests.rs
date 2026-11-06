//! Scoped queue regressions and a reproducible history workload; exercises the store without a server.
//! Called by cargo test; unrelated sessions must cost neither queue slots nor snapshot copies.

use crate::{Change, Delivery, LoadKind, Notification, SessionStore};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn load(store: &SessionStore, id: &str) {
    let _ = store
        .load(
            json!({"session":{"id":id},"history":[],"lastEventId":0}),
            LoadKind::New,
        )
        .unwrap();
}

fn notice(store: &SessionStore, id: Option<&str>, n: usize) {
    let _ = store.apply(Notification::new(
        "warning",
        json!({"sessionId":id,"message":n}),
    ));
}

#[test]
fn scoped_initial_and_lag_snapshots_contain_only_the_followed_session() {
    let store = SessionStore::new();
    load(&store, "a");
    load(&store, "b");
    let sub = store.subscribe_scoped(2, Some("a"));
    let initial = sub.drain();
    assert_eq!(initial.len(), 1);
    assert!(matches!(&initial[0], Delivery::Snapshot(v) if v.session_id == "a"));
    for n in 0..3 {
        notice(&store, Some("a"), n);
    }
    assert_eq!(sub.lag_events(), 1);
    assert_eq!(sub.drain(), initial);
    notice(&store, Some("a"), 3);
    notice(&store, None, 4);
    let items = sub.drain();
    assert!(
        matches!(&items[0], Delivery::Change(Change::Notice { params, .. }) if params["message"] == 3)
    );
    assert!(matches!(
        &items[1],
        Delivery::Change(Change::Notice {
            session_id: None,
            ..
        })
    ));
    let _ = store.remove("a");
    assert!(
        matches!(sub.try_next(), Some(Delivery::Change(Change::Removed { session_id })) if session_id == "a")
    );
}

#[test]
fn unrelated_flood_never_queues_or_wakes_a_scoped_subscriber() {
    let store = SessionStore::new();
    load(&store, "a");
    load(&store, "b");
    let sub = store.subscribe_scoped(2, Some("a"));
    let _ = sub.drain();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    sub.set_waker(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    });
    for n in 0..100 {
        notice(&store, Some("b"), n);
    }
    load(&store, "b");
    assert_eq!(
        sub.lag_events(),
        0,
        "unrelated notices must not consume queue capacity"
    );
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
    assert!(sub.try_next().is_none());
    drop(sub);
    notice(&store, Some("a"), 0);
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
}

#[test]
fn scope_can_follow_a_session_loaded_later() {
    let store = SessionStore::new();
    let sub = store.subscribe_scoped(2, Some("a"));
    load(&store, "b");
    assert!(sub.try_next().is_none());
    load(&store, "a");
    assert!(matches!(sub.try_next(), Some(Delivery::Snapshot(v)) if v.session_id == "a"));
}

#[test]
#[ignore = "manual timing workload: 1000 entries of 4096 bytes, ten subscribers"]
fn scoped_history_workload() {
    for count in [1, 10, 20] {
        let store = SessionStore::new();
        for id in 0..count {
            let history: Vec<_> = (0..1000)
                .map(|entry| json!({"id":entry.to_string(),"text":"x".repeat(4096)}))
                .collect();
            let _ = store
                .load(
                    json!({"session":{"id":id.to_string()},"history":history}),
                    LoadKind::New,
                )
                .unwrap();
        }
        let start = std::time::Instant::now();
        let subs: Vec<_> = (0..10)
            .map(|_| store.subscribe_scoped(2, Some("0")))
            .collect();
        let elapsed = start.elapsed();
        let snapshots: usize = subs.iter().map(|sub| sub.drain().len()).sum();
        println!(
            "sessions={count} subscriptions=10 elapsed_ms={:.3} queued_snapshots={snapshots} copied_text_bytes={}",
            elapsed.as_secs_f64() * 1000.0,
            snapshots * 1000 * 4096
        );
    }
}
