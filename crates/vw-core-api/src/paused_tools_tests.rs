//! The paused-tool module on its own: hold until Resume, Stop and turn end never run the call, the deadline answers
//! unrun and keeps the pause, another chat or tool is not held, the waiting flag follows the waiter.
//!
//! PREDICT (written before the module existed): every test here fails to build, because no pause state exists; with a
//! module that only records the flag (no waiter wake-up), `hold_waits_until_resume` stays pending past Resume and
//! `stop_while_held_never_runs` returns Resumed after a later Resume.
use super::*;
use std::sync::Arc;

#[tokio::test]
async fn not_paused_does_not_wait() {
    let tools = PausedTools::default();
    assert_eq!(
        tools.hold("chat", BROWSER, Duration::from_secs(5)).await,
        Hold::NotPaused
    );
}

#[tokio::test]
async fn hold_waits_until_resume_and_notes_the_first_result() {
    let tools = Arc::new(PausedTools::default());
    assert!(tools.pause("chat", BROWSER, "Chrome"));
    let held = tokio::spawn({
        let tools = tools.clone();
        async move { tools.hold("chat", BROWSER, Duration::from_secs(30)).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!held.is_finished(), "a paused call must wait");
    let view = tools.view("chat");
    assert_eq!(view.paused.len(), 1);
    assert!(view.paused[0].waiting, "the line says the agent is waiting");
    assert_eq!(view.paused[0].place, "Chrome");
    assert!(tools.resume("chat", None));
    assert_eq!(held.await.unwrap(), Hold::Resumed);
    assert_eq!(tools.view("chat").paused.len(), 0);
    assert_eq!(tools.take_resumed_note("chat", BROWSER), Some(RESUMED_NOTE));
    assert_eq!(tools.take_resumed_note("chat", BROWSER), None, "once only");
}

#[tokio::test]
async fn stop_while_held_never_runs_even_after_a_later_resume() {
    let tools = Arc::new(PausedTools::default());
    tools.pause("chat", BROWSER, "Chrome");
    let held = tokio::spawn({
        let tools = tools.clone();
        async move { tools.hold("chat", BROWSER, Duration::from_secs(30)).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(tools.stop_chat("chat"));
    assert!(!tools.resume("chat", None), "nothing left to resume");
    assert_eq!(held.await.unwrap(), Hold::Stopped);
    assert_eq!(tools.take_resumed_note("chat", BROWSER), None);
}

#[tokio::test]
async fn turn_end_ends_the_wait_unrun() {
    let tools = Arc::new(PausedTools::default());
    tools.pause("chat", BROWSER, "Chrome");
    let held = tokio::spawn({
        let tools = tools.clone();
        async move { tools.hold("chat", BROWSER, Duration::from_secs(30)).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(tools.turn_ended("chat"));
    assert_eq!(held.await.unwrap(), Hold::TurnEnded);
    assert!(!tools.is_paused("chat", BROWSER));
}

#[tokio::test(start_paused = true)]
async fn deadline_ends_the_wait_and_keeps_the_pause() {
    let tools = PausedTools::default();
    tools.pause("chat", BROWSER, "Chrome");
    assert_eq!(
        tools.hold("chat", BROWSER, DEFAULT_WAIT).await,
        Hold::TimedOut
    );
    assert!(
        tools.is_paused("chat", BROWSER),
        "the person is still using it"
    );
    assert!(
        !tools.view("chat").paused[0].waiting,
        "nobody waits any more"
    );
}

#[tokio::test]
async fn another_chat_or_tool_is_not_held() {
    let tools = PausedTools::default();
    tools.pause("chat", BROWSER, "Chrome");
    assert_eq!(
        tools.hold("other", BROWSER, Duration::from_secs(5)).await,
        Hold::NotPaused
    );
    assert_eq!(
        tools.hold("chat", "terminal", Duration::from_secs(5)).await,
        Hold::NotPaused
    );
}

#[tokio::test]
async fn a_dropped_wait_clears_the_waiting_flag() {
    let tools = Arc::new(PausedTools::default());
    tools.pause("chat", BROWSER, "Chrome");
    let held = tokio::spawn({
        let tools = tools.clone();
        async move { tools.hold("chat", BROWSER, Duration::from_secs(30)).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(tools.view("chat").paused[0].waiting);
    held.abort();
    let _ = held.await;
    assert!(!tools.view("chat").paused[0].waiting);
    assert!(tools.is_paused("chat", BROWSER));
}

#[test]
fn a_second_pause_changes_nothing() {
    let tools = PausedTools::default();
    assert!(tools.pause("chat", BROWSER, "Chrome"));
    assert!(!tools.pause("chat", BROWSER, "Edge"));
    assert_eq!(tools.view("chat").paused[0].place, "Chrome");
}

#[test]
fn a_note_never_used_ends_with_its_turn() {
    let tools = PausedTools::default();
    tools.pause("chat", BROWSER, "Chrome");
    tools.resume("chat", None);
    tools.turn_ended("chat");
    assert_eq!(tools.take_resumed_note("chat", BROWSER), None);
}
