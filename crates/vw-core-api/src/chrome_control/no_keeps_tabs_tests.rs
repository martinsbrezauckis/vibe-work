//! A person's No on a card declines that one action and closes nothing: the chat's browser session gets the same plain
//! stop as a person's Stop, and the tabs the agent opened stay (No is for the action, not for
//! doing separate things; Stop keeps them too). Uses the offline core and a scripted private owner.
//!
//! PREDICT: on the 0.1.12 line before this change the No path (`ask::root_no` -> `stops::declined`) sent
//! `{"action":"stop","sessionId":…,"closeOwnedTabs":true}`, so the helper closed the agent's tabs.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::test_support;

async fn chat_with_browser_work(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, root| {
        config.chrome_control.package = Some(root.join("package"));
    })
    .await;
    let chrome = &core.shared.chrome_control;
    chrome
        .work
        .lock()
        .unwrap()
        .open("chat-work", "chat", "active", "chrome:p", Some("turn"));
    chrome
        .tasks
        .lock()
        .unwrap()
        .insert("chat-work".into(), "chat".into());
    core
}

/// The stop calls the scripted owner received, waiting (bounded) until at least one arrived.
async fn stops(calls: &Arc<Mutex<Vec<(String, Value)>>>) -> Vec<Value> {
    let read = || -> Vec<Value> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, args)| tool == "chrome_session" && args["action"] == "stop")
            .map(|(_, args)| args.clone())
            .collect()
    };
    let until = Instant::now() + Duration::from_secs(10);
    while read().is_empty() && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    read()
}

#[tokio::test]
async fn a_no_on_a_card_closes_no_tab() {
    let core = chat_with_browser_work("no-keeps-tabs-no").await;
    let calls = test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"state":"stopping"}}),
    );
    let _ = crate::ask::root_no(&core.shared, "chat", async {
        Ok::<_, crate::CoreError>(())
    })
    .await;
    assert_eq!(
        stops(&calls).await,
        [json!({"action":"stop","sessionId":"chat-work"})]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_persons_stop_closes_no_tab() {
    let core = chat_with_browser_work("no-keeps-tabs-stop").await;
    let calls = test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"state":"stopping"}}),
    );
    let _ = crate::stops::requested(
        &core.shared,
        &crate::Command::StopSession {
            session_id: "chat".into(),
        },
    );
    assert_eq!(
        stops(&calls).await,
        [json!({"action":"stop","sessionId":"chat-work"})]
    );
    core.shutdown().await;
}
