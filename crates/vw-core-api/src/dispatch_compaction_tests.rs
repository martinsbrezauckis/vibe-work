//! A chat's Stop and a `session/compact` forward that holds the chat's send lock, on a core without Vibe, with
//! Tokio's clock paused so any wait shows as elapsed time.
//!
//! PREDICT: `stop_lock` re-checked "a compaction holds the lock" on a 50 ms timer, so a Stop already waiting when
//! the compaction is noted let go only after that timer: the first case reads 50 ms of elapsed time before the
//! change and none after it (a compaction start now wakes the waiting Stop). The other cases pin what stays the same.
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
    time::Duration,
};

use tokio::time::Instant;

fn pending(future: Pin<&mut impl Future>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

#[tokio::test(start_paused = true)]
async fn a_waiting_stop_lets_go_as_soon_as_a_compaction_is_noted() {
    let core = crate::offline_core::start("stop-lock-compaction-noted").await;
    let shared = &core.shared;
    let held = shared.plugin_refresh.lock("s").await;
    let stop = super::stop_lock(shared, "s");
    tokio::pin!(stop);
    pending(stop.as_mut());
    let noted = Instant::now();
    crate::stops::note_compaction(shared, "s", true);
    assert!(stop.await.is_none(), "a compaction holds the lock");
    assert_eq!(
        noted.elapsed(),
        Duration::ZERO,
        "the Stop waited for a timer"
    );
    crate::stops::note_compaction(shared, "s", false);
    drop(held);
}

#[tokio::test(start_paused = true)]
async fn a_stop_during_a_compaction_does_not_wait_and_otherwise_takes_the_lock() {
    let core = crate::offline_core::start("stop-lock-compaction-held").await;
    let shared = &core.shared;
    let held = shared.plugin_refresh.lock("s").await;
    crate::stops::note_compaction(shared, "s", true);
    assert!(super::stop_lock(shared, "s").await.is_none());
    crate::stops::note_compaction(shared, "s", false);
    // No compaction: the Stop waits for the lock and gets it when the holder lets go.
    let stop = super::stop_lock(shared, "s");
    tokio::pin!(stop);
    pending(stop.as_mut());
    drop(held);
    assert!(stop.await.is_some());
    // A compaction of another chat does not release this chat's Stop.
    let held = shared.plugin_refresh.lock("s").await;
    let stop = super::stop_lock(shared, "s");
    tokio::pin!(stop);
    pending(stop.as_mut());
    crate::stops::note_compaction(shared, "other", true);
    pending(stop.as_mut());
    crate::stops::note_compaction(shared, "other", false);
    drop(held);
    assert!(stop.await.is_some());
}
