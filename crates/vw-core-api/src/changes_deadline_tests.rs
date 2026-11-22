//! The review's before-work deadline: the reason the Changes panel shows when preparing the review ends late.
//! Uses a core without Vibe, a real git folder and a hook that holds the review after its baseline was written.
//!
//! PREDICT: the reason said "The folder was too big to record before the chat started." although here the
//! baseline was written at once and only the rest of the preparation (the file watcher's start, simulated by
//! the hold before install) was late; the reason now says preparing the review took too long.
//!
//! PREDICT (two reasons, red before the change): both a late recording and a late start after it said "Preparing the
//! review took too long". Now a hold before the baseline is written says the folder took too long to record, and a
//! hold after it says watching the folder took too long to start.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
use vw_platform::sync::LockRecover;

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

/// Holds the review until the test releases it: before its baseline is written (`.1`), or after (`!.1`).
struct Hold(Mutex<mpsc::Receiver<()>>, bool, Arc<AtomicBool>);
impl Hold {
    fn wait(&self) {
        self.2.store(true, Ordering::SeqCst);
        let _ = self
            .0
            .lock_or_recover()
            .recv_timeout(Duration::from_secs(10));
    }
}
impl crate::ChangesTestHooks for Hold {
    fn before_take(&self, _: &str) {
        if self.1 {
            self.wait();
        } else {
            // Simulate a loaded Windows host taking longer to write the baseline.
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn before_install(&self, _: &str) {
        if !self.1 {
            self.wait();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_late_review_says_which_part_took_too_long() {
    assert_eq!(
        Box::pin(late_reason("after", false)).await,
        "Watching the folder for changes took too long to start before the chat started."
    );
    assert_eq!(
        Box::pin(late_reason("before", true)).await,
        "Recording the folder took too long before the chat started."
    );
}

/// The reason a review gets when it is held before (`before_take`) or after its baseline is written.
async fn late_reason(name: &str, before_take: bool) -> String {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/changes-deadline-{name}-{}",
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
    let reached = Arc::new(AtomicBool::new(false));
    let core = Box::pin(crate::Core::start(
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
            changes_test_hooks: Some(Arc::new(Hold(
                Mutex::new(held),
                before_take,
                reached.clone(),
            ))),
            ..crate::CoreConfig::default()
        },
    ))
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
    let params = json!({"sessionId":"late-session"});
    let mut work = Box::pin(core.changes_before_work(&params));
    // Keep virtual time still until the blocking worker reaches the selected phase. Even the
    // deliberately slow recording above cannot choose this test's outcome by wall-clock speed.
    while !reached.load(Ordering::SeqCst) {
        tokio::select! {
            () = &mut work => panic!("review finished before reaching the hold"),
            () = tokio::task::yield_now() => {}
        }
    }
    tokio::time::advance(Duration::from_millis(51)).await;
    work.await;
    let reason = {
        let tracker = core.shared.changes.tracker("late-session").unwrap();
        let guard = tracker.lock_or_recover();
        match &guard.review {
            Some(super::Review::Unavailable(reason)) => reason.clone(),
            _ => panic!("the late review is not unavailable"),
        }
    };
    release.send(()).unwrap();
    core.shutdown().await;
    let _ = std::fs::remove_dir_all(&base);
    reason
}
