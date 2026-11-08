//! Browser activity windows ignore wall-clock corrections while receipt timestamps keep epoch dates.
//! Depends on Sessions' injected wall clock and Tokio time; called by the browser crate's unit tests.
use super::{ACTIVE_FOR_MS, Actor, Outcome, Sessions};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[tokio::test]
async fn a_finished_turn_clears_activity_and_notifies_the_toolbar() {
    let sessions = Sessions::default();
    let flight = sessions.begin("s", Actor::Agent);
    assert!(sessions.queue_native_click("s"));
    let mut changes = sessions.changes();
    sessions.cancel_confirmations("s");
    assert!(!sessions.state("s").agent_active);
    assert!(!sessions.state("s").in_flight);
    assert!(sessions.state("s").pending_native_click.is_none());
    assert_eq!(changes.try_recv().unwrap(), "s");
    // An action already unwinding cannot restore the released turn's indicator.
    sessions.end("s", Actor::Agent, "click", "Save", Outcome::Done, "done");
    drop(flight);
    assert!(!sessions.state("s").agent_active);
    sessions.clear_native_click("s");
    let next = sessions.begin("s", Actor::Agent);
    assert!(sessions.state("s").agent_active);
    drop(next);
}

#[tokio::test(start_paused = true)]
async fn activity_window_ignores_wall_clock_steps() {
    static WALL: AtomicU64 = AtomicU64::new(1_800_000_000_000);
    fn wall() -> u64 {
        WALL.load(Ordering::SeqCst)
    }
    let mut windows = Vec::new();
    for forward in [true, false] {
        WALL.store(1_800_000_000_000, Ordering::SeqCst);
        let sessions = Sessions {
            wall_now: wall,
            ..Sessions::default()
        };
        let flight = sessions.begin("s", Actor::Agent);
        sessions.end("s", Actor::Agent, "click", "Save", Outcome::Done, "done");
        drop(flight);
        let stamped = sessions.state("s");
        assert_eq!(stamped.last_agent_at_ms, Some(wall()));
        assert_eq!(stamped.log[0].at_ms, wall());
        tokio::time::advance(Duration::from_secs(1)).await;
        WALL.store(
            if forward {
                wall() + 61_000
            } else {
                wall() - 59_000
            },
            Ordering::SeqCst,
        );
        let fresh = sessions.state("s").agent_active;
        tokio::time::advance(Duration::from_millis(ACTIVE_FOR_MS)).await;
        WALL.fetch_add(ACTIVE_FOR_MS, Ordering::SeqCst);
        let expired = !sessions.state("s").agent_active;
        windows.push((fresh, expired));
        assert_eq!(
            sessions.state("s").last_agent_at_ms,
            stamped.last_agent_at_ms
        );
    }
    assert_eq!(
        windows,
        [(true, true), (true, true)],
        "clock steps must neither shorten nor extend agent activity"
    );
}
