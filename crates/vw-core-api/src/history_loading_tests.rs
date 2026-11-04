//! Background-history regressions through the normal core read/open path and a deterministic fake Vibe.
//! Depends on `history_test_vibe`, the real gate/store/subscriptions; no GUI or live server.
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::json;
use vw_session::Delivery;

use crate::history_test_vibe::rig;

async fn wait_for(mut ready: impl FnMut() -> bool) {
    // Byte accounting adds work to large histories; allow contention from the full workspace suite.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("background history did not settle");
}

async fn read(core: &crate::Core, id: &str) {
    core.execute(
        crate::Origin::Ui,
        crate::Command::RefreshSession {
            session_id: id.into(),
        },
    )
    .await
    .unwrap();
}

async fn complete(core: &crate::Core, id: &str) {
    wait_for(|| {
        core.shared
            .store
            .view(id)
            .is_some_and(|v| v.state["historyBeforeCursor"].is_null())
    })
    .await;
}

#[tokio::test]
async fn opening_a_saved_chat_loads_the_earliest_prompt_once_in_order() {
    let (core, fake) = rig("history-cold-read", 2202).await;
    let sub = core.shared.store.subscribe(32);
    let started = Instant::now();
    let reply = core
        .execute(
            crate::Origin::Ui,
            crate::Command::RefreshSession {
                session_id: "saved".into(),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = reply else {
        panic!("expected session")
    };
    assert_eq!(
        snapshot.state["history"].as_array().unwrap().len(),
        200,
        "the newest page is available immediately"
    );
    let complete = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let view = core.shared.store.view("saved").unwrap();
            if view.state["historyBeforeCursor"].is_null() {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        complete.is_ok(),
        "earliest prompt never loaded; store still has {} entries",
        core.shared.store.view("saved").unwrap().state["history"]
            .as_array()
            .unwrap()
            .len()
    );
    let state = complete.unwrap().state;
    assert_eq!(
        state["history"],
        json!(fake.entries),
        "every entry appears once, oldest first"
    );
    let last = sub
        .drain()
        .into_iter()
        .filter_map(|d| match d {
            Delivery::Snapshot(v) => Some(v),
            Delivery::Change(_) => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(last.state["history"], state["history"]);
    eprintln!(
        "loaded 2202 entries in {:?} ({} backward requests)",
        started.elapsed(),
        fake.calls.lock().unwrap().len()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn overlaps_keep_live_updates_and_one_loader_per_chat() {
    let (core, fake) = rig("history-live-tail", 2202).await;
    fake.hold.store(true, Ordering::SeqCst);
    fake.overlap.store(true, Ordering::SeqCst);
    read(&core, "saved").await;
    wait_for(|| !fake.calls.lock().unwrap().is_empty()).await;
    for _ in 0..8 {
        core.shared.history_loading.start(&core.shared, "saved");
    }
    let live = json!({"id":"live", "type":"message", "role":"assistant", "content":[{"type":"text","text":"Live answer"}]});
    let _ = core.shared.store.apply(vw_session::Notification::new(
        "history/entryAdded",
        json!({"sessionId":"saved","eventId":1,"entry":live}),
    ));
    let _ = core.shared.store.apply(vw_session::Notification::new("history/entryUpdated", json!({"sessionId":"saved","eventId":2,"entryId":"entry-2002","patch":[{"op":"replace","path":"/message","value":"Updated browser result"}]})));
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
    fake.release.add_permits(16);
    complete(&core, "saved").await;
    let view = core.shared.store.view("saved").unwrap();
    let mut expected = fake.entries.clone();
    expected[2002]["message"] = json!("Updated browser result");
    expected.push(live);
    assert_eq!(view.state["history"], json!(expected));
    assert_eq!(
        view.last_event_id, 2,
        "paging never changes the event watermark"
    );
    assert_eq!(fake.calls.lock().unwrap().len(), 5);
    core.shutdown().await;
}

#[tokio::test]
async fn short_and_already_loaded_chats_do_not_refetch_and_export_is_unchanged() {
    let (short, fake_short) = rig("history-short", 12).await;
    read(&short, "short").await;
    tokio::task::yield_now().await;
    assert!(fake_short.calls.lock().unwrap().is_empty());
    assert_eq!(
        short.shared.store.view("short").unwrap().state,
        fake_short.state("short")
    );
    short.shutdown().await;

    let (core, fake) = rig("history-loaded-export", 2202).await;
    read(&core, "saved").await;
    complete(&core, "saved").await;
    read(&core, "saved").await;
    crate::sessions::observe(
        &core.shared,
        "session/resume",
        &json!({"sessionId":"saved"}),
        &json!({"state":fake.state("saved")}),
    )
    .await
    .unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), 5);
    let before = core.shared.store.view("saved").unwrap();
    let (export, cut) = core.full_history("saved").await.unwrap();
    assert_eq!(export, fake.entries);
    assert!(!cut);
    assert_eq!(core.shared.store.view("saved").unwrap(), before);
    assert_eq!(fake.calls.lock().unwrap().len(), 10);
    core.shutdown().await;
}

#[tokio::test]
async fn subscription_receives_full_history_and_filters_other_chats() {
    let (core, fake) = rig("history-subscription", 2202).await;
    let mut sub = core.subscribe(Some("saved".into()));
    read(&core, "other").await;
    read(&core, "saved").await;
    complete(&core, "saved").await;
    let snapshots = std::iter::from_fn(|| sub.try_next())
        .filter_map(|event| match event {
            crate::CoreEvent::Snapshot { snapshot } => Some(snapshot),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        snapshots
            .iter()
            .all(|snapshot| snapshot.session_id == "saved")
    );
    assert!(
        snapshots
            .windows(2)
            .all(|pair| pair[0].state["historyVersion"].as_u64()
                < pair[1].state["historyVersion"].as_u64())
    );
    assert_eq!(
        snapshots.last().unwrap().state["history"],
        json!(fake.entries)
    );
    core.shutdown().await;
}

#[tokio::test]
async fn resume_import_and_pump_recovery_load_the_returned_sessions_history() {
    for (name, method, source, actual) in [
        ("history-resume", "session/resume", "saved", "saved"),
        ("history-import", "session/resume", "legacy", "imported"),
        ("history-continue", "session/continue", "saved", "saved"),
    ] {
        let (core, fake) = rig(name, 2202).await;
        crate::sessions::observe(
            &core.shared,
            method,
            &json!({"sessionId":source}),
            &json!({"state":fake.state(actual)}),
        )
        .await
        .unwrap();
        complete(&core, actual).await;
        assert_eq!(
            core.shared.store.view(actual).unwrap().state["history"],
            json!(fake.entries)
        );
        assert!(
            fake.calls
                .lock()
                .unwrap()
                .iter()
                .all(|p| p["sessionId"] == actual)
        );
        core.shutdown().await;
    }
    let (core, fake) = rig("history-pump-read", 2202).await;
    let applied = crate::pump::refresh(&core.shared, "saved").await.unwrap();
    core.shared.loads.send(applied).await.unwrap();
    complete(&core, "saved").await;
    assert_eq!(
        core.shared.store.view("saved").unwrap().state["history"],
        json!(fake.entries)
    );
    core.shutdown().await;
}

#[tokio::test]
async fn replacement_close_and_delete_discard_a_pending_page() {
    for action in [
        "session/rewind",
        "session/history/clear",
        "session/compact",
        "close",
        "delete",
    ] {
        let (core, fake) = rig(
            &format!("history-cancel-{}", action.replace('/', "-")),
            2202,
        )
        .await;
        fake.hold.store(true, Ordering::SeqCst);
        read(&core, "saved").await;
        wait_for(|| !fake.calls.lock().unwrap().is_empty()).await;
        if action == "close" {
            core.shared.channels.close("saved").await;
        } else if action == "delete" {
            crate::session_end::deleted(&core.shared, "saved").await;
        } else {
            let mut replacement = fake.state("saved");
            replacement["history"] =
                json!([{"id":"replacement","type":"notice","message":"Replaced"}]);
            replacement["historyBeforeCursor"] = serde_json::Value::Null;
            crate::sessions::observe(
                &core.shared,
                action,
                &json!({"sessionId":"saved"}),
                &json!({"state":replacement}),
            )
            .await
            .unwrap();
        }
        fake.release.add_permits(16);
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(fake.calls.lock().unwrap().len(), 1, "{action}");
        let view = core.shared.store.view("saved");
        match action {
            "delete" => assert!(view.is_none()),
            "close" => assert_eq!(
                view.unwrap().state["history"].as_array().unwrap().len(),
                200
            ),
            _ => assert_eq!(view.unwrap().state["history"][0]["id"], "replacement"),
        }
        core.shutdown().await;
    }
}

#[tokio::test]
async fn history_errors_retry_once_then_stop_without_a_truncation_notice() {
    for failures in [1, 2] {
        let (core, fake) = rig(&format!("history-retry-{failures}"), 2202).await;
        fake.failures.store(failures, Ordering::SeqCst);
        read(&core, "saved").await;
        if failures == 1 {
            complete(&core, "saved").await;
            assert_eq!(fake.calls.lock().unwrap().len(), 6);
        } else {
            wait_for(|| fake.calls.lock().unwrap().len() == 2).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            core.shared.history_loading.start(&core.shared, "saved");
            tokio::task::yield_now().await;
            assert_eq!(fake.calls.lock().unwrap().len(), 2);
            assert_eq!(
                core.shared.store.view("saved").unwrap().state["history"]
                    .as_array()
                    .unwrap()
                    .len(),
                200
            );
        }
        assert_ne!(
            core.shared.store.view("saved").unwrap().state["historyTruncated"],
            true
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_repeated_cursor_stops_after_one_retry() {
    let (core, fake) = rig("history-bad-cursor", 2202).await;
    fake.repeated_cursor.store(true, Ordering::SeqCst);
    read(&core, "saved").await;
    wait_for(|| fake.calls.lock().unwrap().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    assert_eq!(
        core.shared.store.view("saved").unwrap().state["history"]
            .as_array()
            .unwrap()
            .len(),
        200
    );
    core.shutdown().await;
}

#[tokio::test]
async fn only_the_safety_cap_marks_history_truncated() {
    for count in [20_000, 20_202] {
        let (core, fake) = rig(&format!("history-cap-{count}"), count).await;
        read(&core, "saved").await;
        wait_for(|| {
            core.shared.store.view("saved").unwrap().state["history"]
                .as_array()
                .unwrap()
                .len()
                == 20_000
        })
        .await;
        let view = core.shared.store.view("saved").unwrap();
        assert_eq!(
            view.state["historyTruncated"].as_bool().unwrap_or(false),
            count > 20_000
        );
        assert_eq!(view.state["history"], json!(fake.entries[count - 20_000..]));
        let calls = fake.calls.lock().unwrap().len();
        read(&core, "saved").await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            fake.calls.lock().unwrap().len(),
            calls,
            "the cap survives a warm read"
        );
        assert_eq!(
            core.shared.store.view("saved").unwrap().state["historyTruncated"]
                .as_bool()
                .unwrap_or(false),
            count > 20_000
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn large_history_pages_stop_at_the_byte_budget_and_preserve_the_live_tail() {
    const BUDGET: usize = 64 * 1024 * 1024;
    // Twenty available pages below the RPC frame cap, generated lazily to keep the fixture small.
    let (core, fake) = rig("history-byte-budget", 200 + 20 * 50).await;
    fake.page_size.store(50, Ordering::SeqCst);
    fake.text_bytes.store(1024 * 1024, Ordering::SeqCst);
    fake.hold.store(true, Ordering::SeqCst);
    fake.release.add_permits(2);
    read(&core, "saved").await;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view = core.shared.store.view("saved").unwrap();
            if view.state["historyTruncated"] == true
                || view.state["history"].as_array().unwrap().len() >= 300
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let view = core.shared.store.view("saved").unwrap();
    let bytes = serde_json::to_vec(&view.state["history"]).unwrap().len();
    eprintln!("retained history: {bytes} bytes; budget: {BUDGET}");
    assert!(bytes <= BUDGET, "resident history exceeded the byte budget");
    assert_eq!(view.state["historyTruncated"], true);
    assert_eq!(view.state["history"].as_array().unwrap().len(), 263);
    assert_eq!(view.state["history"][0]["id"], "entry-937");
    drop(view);
    fake.release.add_permits(20);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    read(&core, "saved").await;
    assert_eq!(
        fake.calls.lock().unwrap().len(),
        2,
        "warm reads keep the cap"
    );
    let _ = core.shared.store.apply(vw_session::Notification::new(
        "history/entryAdded",
        json!({"sessionId":"saved","eventId":1,"entry":{"id":"live","message":"y".repeat(2 * 1024 * 1024)}}),
    ));
    let view = core.shared.store.view("saved").unwrap();
    assert_eq!(
        view.state["history"].as_array().unwrap().last().unwrap()["id"],
        "live"
    );
    assert!(serde_json::to_vec(&view.state["history"]).unwrap().len() > BUDGET);
    core.shutdown().await;
}

#[tokio::test]
async fn five_chats_share_two_loader_slots_and_waiting_close_or_delete_cancels() {
    let (core, fake) = rig("history-global-slots", 250).await;
    fake.hold.store(true, Ordering::SeqCst);
    tokio::join!(
        read(&core, "first"),
        read(&core, "second"),
        read(&core, "close-waiting"),
        read(&core, "delete-waiting"),
        read(&core, "last"),
    );
    wait_for(|| fake.active.load(Ordering::SeqCst) >= 2).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(fake.active.load(Ordering::SeqCst), 2);
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    let running: Vec<String> = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|p| p["sessionId"].as_str().unwrap().to_owned())
        .collect();
    let waiting: Vec<&str> = ["first", "second", "close-waiting", "delete-waiting", "last"]
        .into_iter()
        .filter(|id| !running.iter().any(|s| s == id))
        .collect();
    let (close, delete, last) = (waiting[0], waiting[1], waiting[2]);
    // Slots stay held until below; a generous timeout still proves cancellation never waits for one.
    tokio::time::timeout(Duration::from_secs(10), async {
        core.shared.channels.close(close).await;
        crate::session_end::deleted(&core.shared, delete).await;
    })
    .await
    .expect("cancelling a waiting loader must not wait for a slot");
    assert!(core.shared.store.history_request(close).is_none());
    assert!(core.shared.store.view(delete).is_none());
    fake.release.add_permits(3);
    complete(&core, &running[0]).await;
    complete(&core, &running[1]).await;
    complete(&core, last).await;
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|p| p["sessionId"] != close && p["sessionId"] != delete)
    );
    assert_eq!(fake.peak.load(Ordering::SeqCst), 2);
    assert_eq!(
        core.shared.store.view(close).unwrap().state["history"]
            .as_array()
            .unwrap()
            .len(),
        200
    );
    core.shutdown().await;
}

#[tokio::test]
async fn cancelling_an_active_loader_releases_a_slot_and_revalidates_queued_generations() {
    let (core, fake) = rig("history-slot-cancel", 250).await;
    fake.hold.store(true, Ordering::SeqCst);
    read(&core, "first").await;
    read(&core, "second").await;
    wait_for(|| fake.active.load(Ordering::SeqCst) == 2).await;
    read(&core, "replaced").await;
    read(&core, "waiting").await;
    // Simulate a replacement before its new loader is started. The old queued generation must not read.
    let replacement = fake.state("replaced");
    let _ = core
        .shared
        .store
        .load(replacement, vw_session::LoadKind::Replace)
        .unwrap();
    core.shared.channels.close("first").await;
    wait_for(|| fake.calls.lock().unwrap().len() == 3).await;
    assert_eq!(fake.calls.lock().unwrap()[2]["sessionId"], "waiting");
    assert_eq!(fake.active.load(Ordering::SeqCst), 2);
    fake.release.add_permits(2);
    complete(&core, "second").await;
    complete(&core, "waiting").await;
    assert_eq!(fake.peak.load(Ordering::SeqCst), 2);
    assert_eq!(
        core.shared.store.view("replaced").unwrap().state["history"]
            .as_array()
            .unwrap()
            .len(),
        200
    );
    core.shutdown().await;
}
