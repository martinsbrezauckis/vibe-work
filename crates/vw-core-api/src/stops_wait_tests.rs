//! [`super::until_ended`], the wait a running video job or secret capture holds until its chat is stopped, removed
//! or loses access, on a core without Vibe and with Tokio's clock paused, so any wait shows as elapsed time.
//!
//! PREDICT: both waits re-checked their conditions on a 25 ms timer. The Stop, failed-turn and removed-chat cases
//! then read 25 ms of elapsed time instead of none, and the idle case counts 41 checks in one second instead of at
//! most five (one at the start and one per [`super::RECHECK`]). A condition no event announces is still seen within
//! one re-check either way.
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Waker},
    time::Duration,
};

use serde_json::json;
use tokio::time::Instant;

fn pending(future: Pin<&mut impl Future>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

#[tokio::test(start_paused = true)]
async fn a_stop_ends_the_wait_at_once() {
    let core = crate::offline_core::start("stop-wait-stop").await;
    let shared = &core.shared;
    let before = super::generation(shared, "s");
    let wait = super::until_ended(shared, || super::generation(shared, "s") != before);
    tokio::pin!(wait);
    pending(wait.as_mut());
    let stopped = Instant::now();
    super::stopped(shared, "s");
    wait.await;
    assert_eq!(stopped.elapsed(), Duration::ZERO, "the wait ran a timer");
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_ends_the_wait_at_once() {
    let core = crate::offline_core::start("stop-wait-failed").await;
    let shared = &core.shared;
    let before = super::generation(shared, "s");
    let wait = super::until_ended(shared, || super::generation(shared, "s") != before);
    tokio::pin!(wait);
    pending(wait.as_mut());
    let failed = Instant::now();
    super::observe(
        shared,
        &vw_session::Change::TurnUpserted {
            session_id: "s".into(),
            turn: json!({"id": "t1", "status": "failed"}),
        },
    );
    wait.await;
    assert_eq!(failed.elapsed(), Duration::ZERO, "the wait ran a timer");
}

#[tokio::test(start_paused = true)]
async fn a_removed_chat_ends_the_wait_at_once() {
    let core = crate::offline_core::start("stop-wait-removed").await;
    let shared = &core.shared;
    let removed = AtomicBool::new(false);
    let wait = super::until_ended(shared, || removed.load(Ordering::SeqCst));
    tokio::pin!(wait);
    pending(wait.as_mut());
    let at = Instant::now();
    removed.store(true, Ordering::SeqCst);
    super::observe(
        shared,
        &vw_session::Change::Removed {
            session_id: "s".into(),
        },
    );
    wait.await;
    assert_eq!(at.elapsed(), Duration::ZERO, "the wait ran a timer");
}

#[tokio::test(start_paused = true)]
async fn a_condition_without_an_event_is_seen_within_one_recheck() {
    let core = crate::offline_core::start("stop-wait-quiet").await;
    let shared = &core.shared;
    let ended = AtomicBool::new(false);
    let wait = super::until_ended(shared, || ended.load(Ordering::SeqCst));
    tokio::pin!(wait);
    pending(wait.as_mut());
    let at = Instant::now();
    // Access or tool registration changed: no Stop event announces it.
    ended.store(true, Ordering::SeqCst);
    wait.await;
    assert!(at.elapsed() <= super::RECHECK, "{:?}", at.elapsed());
}

#[tokio::test(start_paused = true)]
async fn an_idle_wait_checks_rarely_and_another_chats_stop_does_not_end_it() {
    let core = crate::offline_core::start("stop-wait-idle").await;
    let shared = &core.shared;
    let checks = Arc::new(AtomicUsize::new(0));
    let counted = checks.clone();
    let before = super::generation(shared, "s");
    let wait = super::until_ended(shared, move || {
        counted.fetch_add(1, Ordering::SeqCst);
        super::generation(shared, "s") != before
    });
    tokio::pin!(wait);
    let idle = tokio::time::timeout(Duration::from_secs(1), wait.as_mut()).await;
    assert!(idle.is_err(), "the wait ended without a Stop");
    let counted = checks.load(Ordering::SeqCst);
    assert!(counted <= 5, "{counted} checks in one idle second");
    super::stopped(shared, "other");
    pending(wait.as_mut());
    super::stopped(shared, "s");
    wait.await;
}
