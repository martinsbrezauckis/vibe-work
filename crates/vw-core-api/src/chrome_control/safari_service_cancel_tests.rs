//! A blocked native clipboard read cannot delay Safari Stop; a cancelled queued write never reaches the backend.
use super::*;
use std::sync::{atomic::AtomicUsize, mpsc};
use vw_platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardService};
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
struct Blocked {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: mpsc::Receiver<()>,
    writes: Arc<AtomicUsize>,
}
impl ClipboardBackend for Blocked {
    fn get_text(&mut self) -> Result<String, ClipboardError> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            let _ = self.release.recv();
        }
        Ok("ordinary text".into())
    }
    fn set_text(&mut self, _: &str) -> Result<(), ClipboardError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn clear(&mut self) -> Result<(), ClipboardError> {
        Ok(())
    }
}
#[tokio::test]
async fn safari_stop_drops_a_blocked_clipboard_wait_and_cancels_queued_writes() {
    blocked_clipboard("clipboard", "clipboard.read", json!({}), false).await;
}
#[tokio::test]
async fn native_paste_drops_a_blocked_clipboard_guard_on_stop() {
    blocked_clipboard(
        "native",
        "page.key",
        json!({"key":"v","modifiers":["Meta"]}),
        false,
    )
    .await;
}
#[tokio::test]
async fn native_paste_wait_is_cancelled_when_a_person_question_opens() {
    blocked_clipboard(
        "native",
        "page.key",
        json!({"key":"v","modifiers":["Meta"]}),
        true,
    )
    .await;
}
async fn blocked_clipboard(kind: &str, action: &str, args: Value, question: bool) {
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, blocked) = mpsc::channel();
    let release = Release(Some(release));
    let writes = Arc::new(AtomicUsize::new(0));
    let service = Arc::new(
        ClipboardService::with_backend(
            Box::new(Blocked {
                entered: Some(entered),
                release: blocked,
                writes: writes.clone(),
            }),
            std::time::Duration::from_secs(30),
        )
        .unwrap(),
    );
    let core = crate::offline_core::start_with(&format!("safari-{kind}-stop"), |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.clipboard = Some(service.clone());
    })
    .await;
    let shared = &core.shared;
    assert!(
        shared
            .store
            .load(
                json!({"session":{"id":"chat"},"history":[],"turns":[]}),
                vw_session::LoadKind::New
            )
            .is_ok()
    );
    shared.access.observe_request(
        "session/agent/update",
        "chat",
        &json!({"agentName":"auto-approve"}),
    );
    let dir = shared.chrome_control.dir().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("safari-enabled.json"), b"true").unwrap();
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("task".into(), "chat".into());
    shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("task", "chat", "active", "safari:p", None);
    let live = Arc::new(AtomicBool::new(true));
    let helper_live = live.clone();
    super::super::test_support::answering(shared, move |method, _| {
        assert_eq!(method, "owner.safariService.check");
        json!({"active":helper_live.load(Ordering::Acquire)})
    });
    let cloned = shared.clone();
    let claim = json!({"kind":kind,"stage":"dispatch","action":action,"args":args,"requestId":"r","sessionId":"task","clientSessionId":"chat","profileId":"safari:p"});
    let work = tokio::spawn(async move { run(&cloned, &claim).await });
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    interrupt_service(shared, &live, question);
    let result = tokio::time::timeout(std::time::Duration::from_millis(500), work)
        .await
        .unwrap()
        .unwrap();
    let reason = result.unwrap_err().message;
    assert!(
        reason.contains(if question {
            "user's answer"
        } else {
            "operation, connection or permission ended"
        }),
        "{reason}"
    );
    if question {
        assert!(reason.contains("answer"), "{reason}");
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            service.write_text("late write".into())
        )
        .await
        .is_err()
    );
    drop(release);
    assert_eq!(service.read_text().await.unwrap(), "ordinary text");
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    core.shutdown().await;
    service.shutdown().await;
}

fn interrupt_service(shared: &Shared, live: &AtomicBool, question: bool) {
    if question {
        shared.asks.open(
            "user_input",
            Some("chat".into()),
            vw_ask::Requester::Agent,
            vec![vw_ask::Choice::No],
            Some("person-step".into()),
        );
    } else {
        live.store(false, Ordering::Release);
    }
}
