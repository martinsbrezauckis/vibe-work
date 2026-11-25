//! A Developer-permissions switch between the first chat check and the forwarded send.
//! Uses the real dispatcher and a paused Changes baseline, with a mock Vibe channel recording requests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use serde_json::json;
use vw_platform::sync::LockRecover;

use crate::refresh_notice_tests::{AlwaysAllow, attach_mock_channel};
use crate::{Command, Origin};

struct HoldBaseline {
    entered: Arc<AtomicBool>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl crate::ChangesTestHooks for HoldBaseline {
    fn before_take(&self, _: &str) {
        self.entered.store(true, Ordering::SeqCst);
        let _ = self
            .release
            .lock_or_recover()
            .recv_timeout(Duration::from_secs(15));
    }
}

#[tokio::test]
async fn a_switch_between_before_work_and_send_refuses_old_runtime_work() {
    for (name, origin, method, params) in [
        (
            "turn-ui",
            Origin::Ui,
            "turn/start",
            json!({"sessionId":"race-chat", "message":[{"type":"text","text":"hello"}]}),
        ),
        (
            "queue-ui",
            Origin::Ui,
            "session/turn/enqueue",
            json!({"sessionId":"race-chat", "entries":[]}),
        ),
        (
            "turn-debug",
            Origin::DebugApi,
            "turn/start",
            json!({"sessionId":"race-chat", "message":[{"type":"text","text":"hello"}]}),
        ),
    ] {
        Box::pin(one_send(name, origin, method, params)).await;
    }
}

async fn one_send(name: &str, origin: Origin, method: &str, params: serde_json::Value) {
    let entered = Arc::new(AtomicBool::new(false));
    let (release, waiting) = mpsc::channel();
    let hook = Arc::new(HoldBaseline {
        entered: entered.clone(),
        release: Mutex::new(waiting),
    });
    let core = crate::offline_core::start_with_confirmer(
        &format!("devlazy-forward-send-race-{name}"),
        move |config, dir| {
            let app_data = dir.join("app-data");
            let vibe_home = dir.join("vibe-home");
            std::fs::create_dir_all(&app_data).unwrap();
            std::fs::create_dir_all(&vibe_home).unwrap();
            std::fs::write(app_data.join("settings.json"), r#"{"developerPermissions":true}"#).unwrap();
            std::fs::write(vibe_home.join("config.toml"),
                "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\" }]\n").unwrap();
            config.secret_scan_data = Some(app_data);
            config.connection_wait = Duration::from_millis(20);
            config.changes_dir = Some(dir.join("changes"));
            config.changes_baseline_limit = Duration::from_secs(15);
            config.changes_test_hooks = Some(hook);
            let repo = dir.join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
            std::fs::write(repo.join("file.txt"), "content\n").unwrap();
        },
        AlwaysAllow,
    ).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let _mock = attach_mock_channel(&core, "race-chat", 1, calls.clone());
    let repo = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join("repo");
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"race-chat","cwd":repo,
        "status":{"type":"idle"}},"entries":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();

    let send = core.forward(origin, method, params, None);
    tokio::pin!(send);
    while !entered.load(Ordering::SeqCst) {
        tokio::select! {
            result = &mut send => panic!("send returned before the hold: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
    }
    core.execute(
        Origin::Ui,
        Command::DeveloperPermissions {
            enabled: Some(false),
        },
    )
    .await
    .unwrap();
    assert!(core.shared.channels.switch_stale("race-chat"));
    release.send(()).unwrap();
    let error = send
        .await
        .expect_err("old-runtime work must be refused after OFF");
    assert_eq!(
        error.server_code.as_deref(),
        Some("developerSwitchNotApplied")
    );
    assert!(
        !calls.lock().unwrap().iter().any(|(_, sent)| sent == method),
        "{name}: {method} reached Vibe after OFF"
    );
    core.shutdown().await;
}
