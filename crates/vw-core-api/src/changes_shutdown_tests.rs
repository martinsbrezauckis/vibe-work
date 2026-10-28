//! Shutdown waits for a real background git refresh. Uses a core without Vibe and a held refresh hook.
use std::sync::{Arc, Mutex, mpsc};
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
            panic!("a git refresh must not prompt")
        })
    }
}

struct Hold {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl crate::ChangesTestHooks for Hold {
    fn before_refresh(&self, _: &str) {
        self.entered.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }
}

#[tokio::test]
async fn shutdown_joins_a_refresh_before_its_folder_can_be_removed() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/tmp/changes-shutdown-{}",
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
    let (entered, ready) = mpsc::channel();
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
            changes_test_hooks: Some(Arc::new(Hold {
                entered,
                release: Mutex::new(held),
            })),
            ..crate::CoreConfig::default()
        },
    )
    .await
    .unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"shutdown-session", "cwd":repo}, "entries":[], "turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared
        .changes
        .signals
        .send("shutdown-session".into())
        .unwrap();
    tokio::task::spawn_blocking(move || ready.recv_timeout(Duration::from_secs(10)).unwrap())
        .await
        .unwrap();
    let owner = core.clone();
    let mut stopping = tokio::spawn(async move { owner.shutdown().await });
    let returned_early = tokio::time::timeout(Duration::from_millis(250), &mut stopping)
        .await
        .is_ok();
    release.send(()).unwrap();
    if !returned_early {
        tokio::time::timeout(Duration::from_secs(10), stopping)
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        !returned_early,
        "shutdown returned while the git refresh was still held"
    );
    std::fs::remove_dir_all(&base).unwrap();
    assert!(!base.exists());
}
