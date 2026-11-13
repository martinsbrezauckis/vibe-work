//! The review's before-work deadline: the reason the Changes panel shows when preparing the review ends late.
//! Uses a core without Vibe, a real git folder and a hook that holds the review after its baseline was written.
//!
//! PREDICT: the reason said "The folder was too big to record before the session started." although here the
//! baseline was written at once and only the rest of the preparation (the file watcher's start, simulated by
//! the hold before install) was late; the reason now says preparing the review took too long.
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::poll_fn(|_| -> std::task::Poll<Confirmation> {
            panic!("preparing a review must not prompt")
        })
    }
}

/// Holds the review after its baseline was written until the test releases it.
struct Hold(Mutex<mpsc::Receiver<()>>);
impl crate::ChangesTestHooks for Hold {
    fn before_install(&self, _: &str) {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(Duration::from_secs(10));
    }
}

#[tokio::test]
async fn a_late_review_says_its_preparation_took_too_long() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/changes-deadline-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&base).unwrap();
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(repo.join("file.txt"), "content\n").unwrap();
    let (release, held) = mpsc::channel();
    let core = crate::Core::start(
        vw_supervisor::SupervisorConfig::new(
            base.join("no-server"),
            base.join("home"),
            base.clone(),
        ),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        crate::CoreConfig {
            start_vibe: false,
            changes_dir: Some(base.join("data")),
            changes_baseline_limit: Duration::from_millis(50),
            changes_test_hooks: Some(Arc::new(Hold(Mutex::new(held)))),
            ..crate::CoreConfig::default()
        },
    )
    .await
    .unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"late-session", "cwd":repo}, "entries":[], "turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.changes_before_work(&json!({"sessionId":"late-session"}))
        .await;
    let reason = {
        let tracker = core.shared.changes.tracker("late-session").unwrap();
        let guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
        match &guard.review {
            Some(super::Review::Unavailable(reason)) => reason.clone(),
            _ => panic!("the late review is not unavailable"),
        }
    };
    release.send(()).unwrap();
    core.shutdown().await;
    assert_eq!(
        reason,
        "Preparing the review took too long before the session started."
    );
    let _ = std::fs::remove_dir_all(&base);
}
