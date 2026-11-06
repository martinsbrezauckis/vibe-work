//! Pump recovery under a paused Tokio clock, without a server or further incoming notifications.
//! Exercises the real pump and read-attempt counter; deletion and shutdown must cancel deferred work.

use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;
use vw_session::{Action, Applied, LoadKind};

async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

async fn exhausted(name: &str) -> crate::Core {
    let (core, _fake) = crate::history_test_vibe::rig(name, 0).await;
    tokio::time::pause();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"paused"},"history":[],"turns":[]}),
            LoadKind::Read,
        )
        .unwrap();
    for _ in 0..20 {
        core.shared
            .loads
            .send(Applied {
                changes: vec![],
                actions: vec![Action::NeedsResync {
                    session_id: "paused".into(),
                    reason: "gap".into(),
                    delayed: false,
                }],
            })
            .await
            .unwrap();
        settle().await;
    }
    assert_eq!(core.shared.resync_reads.load(Ordering::Relaxed), 8);
    core
}

#[tokio::test]
async fn budget_expiry_retries_once_without_another_event() {
    let core = exhausted("resync-expiry").await;
    tokio::time::advance(Duration::from_secs(29)).await;
    settle().await;
    assert_eq!(core.shared.resync_reads.load(Ordering::Relaxed), 8);
    // Tokio rounds timer deadlines up to the next millisecond.
    tokio::time::advance(Duration::from_millis(1001)).await;
    settle().await;
    let at_expiry = core.shared.resync_reads.load(Ordering::Relaxed);
    eprintln!(
        "requests=20 reads_before_pause=8 elapsed_virtual_seconds=30.001 reads_at_expiry={at_expiry}"
    );
    tokio::time::advance(Duration::from_secs(90)).await;
    settle().await;
    let later = core.shared.resync_reads.load(Ordering::Relaxed);
    core.shutdown().await;
    assert_eq!(
        at_expiry, 9,
        "one recovery read must run without a new event"
    );
    assert_eq!(
        later, 9,
        "the deferred retry must not become a polling loop"
    );
}

#[tokio::test]
async fn removal_and_shutdown_cancel_paused_recovery() {
    for shutdown in [false, true] {
        let core = exhausted("resync-cancel").await;
        if shutdown {
            core.shutdown().await;
        } else {
            let applied = core.shared.store.remove("paused");
            core.shared.loads.send(applied).await.unwrap();
        }
        settle().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(core.shared.resync_reads.load(Ordering::Relaxed), 8);
        if !shutdown {
            core.shutdown().await;
        }
        tokio::time::resume();
    }
}
