//! Late helper replies race a real chat Stop at its admission lock; another chat keeps its work.
use super::super::{agent_browser::Browser, gateway, owner, person_stop, test_support};
use serde_json::json;
use std::{sync::Arc, time::Duration};

// Stop waits on the display-frame lock after cleanup; the reply waits at the real task admission lock.
fn admit_after_cleanup(
    shared: &Arc<crate::core::Shared>,
    pending: person_stop::PendingCall,
    alias: bool,
    failed: bool,
) -> (Result<(), String>, Option<String>, Option<String>) {
    let frames = shared.chrome_control.frames.lock().unwrap();
    let stop_shared = Arc::clone(shared);
    let stopping = std::thread::spawn(move || {
        crate::stops::stopped(&stop_shared, if alias { "old-a" } else { "a" });
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .contains_key("old-a-work")
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Stop never removed its old task"
        );
        std::thread::yield_now();
    }
    // The open reply is held at precisely the same tasks lock scope_result takes at admission.
    let admission = shared.chrome_control.tasks.lock().unwrap();
    let reply_shared = Arc::clone(shared);
    let (ready, waiting) = std::sync::mpsc::channel();
    let reply = std::thread::spawn(move || {
        let mut answer = owner::Answer::Helper(if failed {
            json!({"isError":true,"structuredContent":{"error":{"code":"OPEN_FAILED"}}})
        } else {
            json!({"structuredContent":{"sessionId":"late-a-work","profileId":"chrome:p","state":"active"}})
        });
        ready.send(()).unwrap();
        gateway::scope_result(
            &reply_shared,
            "a",
            "chrome_session",
            true,
            Browser::Chrome,
            &pending,
            &mut answer,
        )
    });
    waiting.recv_timeout(Duration::from_secs(10)).unwrap();
    drop(admission);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !reply.is_finished()
        && !shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .contains_key("late-a-work")
    {
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::yield_now();
    }
    let residual = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .get("late-a-work")
        .cloned();
    let late_work = shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .state_of("late-a-work", "a")
        .map(str::to_owned);
    drop(frames);
    stopping.join().unwrap();
    let result = reply.join().unwrap();
    (result, residual, late_work)
}

async fn stop_during_open(alias: bool, failed: bool) {
    let core = crate::offline_core::start_with("stop-open-admission", |config, root| {
        config.chrome_control.package = Some(root.join("package"));
    })
    .await;
    let shared = &core.shared;
    for chat in ["a", "b"] {
        test_support::chat(shared, chat);
    }
    if alias {
        shared.channels.imported_as("old-a", "a");
    }
    let calls = test_support::answering(shared, |_, _| json!({"applied":true}));
    for (id, chat) in [
        ("old-a-work", if alias { "old-a" } else { "a" }),
        ("b-work", "b"),
    ] {
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open(id, chat, "active", "chrome:p", None);
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert(id.into(), chat.into());
    }
    let pending = person_stop::PendingCall::new(
        shared,
        "a",
        "chrome_session",
        &json!({"action":"open","profileId":"chrome:p"}),
    );
    let (result, residual, late_work) = admit_after_cleanup(shared, pending, alias, failed);
    // Drain all detached helper messages with a following owner request, preserving their order.
    let _ = shared
        .chrome_control
        .owner
        .rpc("test.barrier", json!({}))
        .await;
    let stops: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, args)| name == "chrome_session" && args["action"] == "stop")
        .map(|(_, args)| args["sessionId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .get("b-work")
            .map(String::as_str),
        Some("b")
    );
    assert_eq!(
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("b-work", "b"),
        Some("active")
    );
    core.shutdown().await;
    assert!(
        result.is_err(),
        "late open was admitted: residual={residual:?}, work={late_work:?}"
    );
    assert_eq!(residual, None);
    assert_eq!(late_work, None);
    assert_eq!(
        stops
            .iter()
            .filter(|id| id.as_str() == "late-a-work")
            .count(),
        usize::from(!failed)
    );
    assert!(!stops.iter().any(|id| id == "b-work"));
}

#[tokio::test]
async fn stop_refuses_an_open_reply_waiting_at_admission() {
    stop_during_open(false, false).await;
}
#[tokio::test]
async fn alias_stop_refuses_an_open_reply_waiting_at_admission() {
    stop_during_open(true, false).await;
}
#[tokio::test]
async fn stop_refuses_a_failed_open_without_stopping_another_chat() {
    stop_during_open(false, true).await;
}
