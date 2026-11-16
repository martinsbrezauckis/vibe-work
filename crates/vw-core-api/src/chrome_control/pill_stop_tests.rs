//! Stop pressed in the helper browser's pill while the agent's call waits for the paused browser.
//!
//! PREDICT (written before the fix): the pill's Stop reaches the app as a stopped helper row, which interrupts the
//! turn. The held call is then ended by whichever of the interrupt and the pause's own Stop answer wins, and the
//! answer it gets is not the paused-browser Stop receipt that the chat-line and composer Stop give; the person's
//! line falls back to the generic "safety reminder refused the call".
use super::person_stop_tests::{fixture, runtime, status};
use crate::paused_tools::BROWSER;
use serde_json::json;
use std::time::Duration;
use tokio::time::Instant;

async fn held_call(core: &crate::Core) -> tokio::task::JoinHandle<(Result<u64, String>, Instant)> {
    core.shared.paused_tools.pause("a", BROWSER, "Chrome");
    let shared = core.shared.clone();
    let held = tokio::spawn(async move {
        let answer = crate::paused_tools_gate::receipt(
            &shared,
            "a",
            "chrome_execute",
            &json!({"sessionId":"task-a","action":"page.navigate","args":{"url":"https://example.org"}}),
            None,
        )
        .await;
        (answer, Instant::now())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!held.is_finished(), "the call waits while paused");
    held
}

/// The pill's Stop ends the held call with the paused-browser Stop receipt, and that answer leaves the app before
/// the interrupt reaches the runtime (an interrupt that wins the race cancels the hook, which then shows the
/// generic safety-reminder line instead of the receipt).
#[tokio::test]
async fn a_pill_stop_ends_the_held_call_with_the_paused_stop_receipt_before_the_interrupt() {
    let core = fixture("pill-stop-held", "chrome:p").await;
    let calls = runtime(&core);
    let held = held_call(&core).await;
    let arrival = tokio::spawn({
        let calls = calls.clone();
        async move {
            loop {
                if !calls.lock().unwrap().is_empty() {
                    return Instant::now();
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    });
    status(&core, "stopping", Instant::now());
    let (answer, answered_at) = tokio::time::timeout(Duration::from_secs(3), held)
        .await
        .expect("the held call ends")
        .unwrap();
    assert_eq!(answer, Err(crate::paused_tools_gate::STOPPED.to_owned()));
    let interrupted_at = tokio::time::timeout(Duration::from_secs(3), arrival)
        .await
        .expect("the interrupt reaches the runtime")
        .unwrap();
    let lead = interrupted_at.saturating_duration_since(answered_at);
    assert!(
        lead >= Duration::from_millis(100),
        "the answer leaves {lead:?} before the interrupt; the hook needs time to deliver it"
    );
    core.shutdown().await;
}
