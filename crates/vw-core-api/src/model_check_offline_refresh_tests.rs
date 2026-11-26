//! Controlled catalog delay for the real pinned Vibe launch-read catalog boundary.
//! Uses loopback HTTP only; dropping the fixture ends its held response.
//! Called by the offline model-check tests to check read ordering and post-refresh Send preservation.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

pub(super) struct DelayedCatalog {
    pub base: String,
    arrived: Arc<Notify>,
    released: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl DelayedCatalog {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let arrived = Arc::new(Notify::new());
        let released = Arc::new(Notify::new());
        let seen = Arc::clone(&arrived);
        let release = Arc::clone(&released);
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut first = [0_u8; 1];
            stream.read_exact(&mut first).await.unwrap();
            seen.notify_one();
            stream
                .write_all(b"HTTP/1.1 404 Not Found\r\n")
                .await
                .unwrap();
            // Keep the per-read HTTP timeout alive without finishing the headers.
            // The launch read must hit its own cap while this fetch is outstanding.
            let mut pulse = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    () = release.notified() => break,
                    _ = pulse.tick() => {
                        if stream.write_all(b"X-Held: catalog\r\n").await.is_err() {
                            return;
                        }
                    }
                }
            }
            stream
                .write_all(b"Content-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });
        Self {
            base,
            arrived,
            released,
            task,
        }
    }

    pub async fn wait_for_request(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.arrived.notified())
            .await
            .unwrap();
    }

    pub fn release(&self) {
        self.released.notify_one();
    }
}

impl Drop for DelayedCatalog {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn preserve_after_refresh(
    core: &crate::Core,
    root: &std::path::Path,
    policy: &str,
) {
    core.register_picked_folder(root).unwrap();
    let mut chats = Vec::new();
    for saved in ["one", "two"] {
        // Full access comes from this fixture's configured default, as it does
        // in the existing real-Vibe access-mode tests, without a new prompt.
        let mut config = json!({"cwd":root,"trustWorkspace":true});
        if policy != "auto-approve" {
            config["approvalPolicy"] = json!(policy);
        }
        let session = core
            .execute(
                crate::Origin::Ui,
                crate::Command::StartSession {
                    agent_config: Some(config),
                },
            )
            .await
            .unwrap();
        let crate::CommandResult::Session { snapshot, .. } = session else {
            panic!("expected a real Vibe session")
        };
        let id = snapshot.session_id;
        let ready = rpc(core, "session/ready/wait", &id).await;
        assert_eq!(ready["ready"], true, "{ready}");
        crate::model_choices::handed_off(&core.shared.model_choices, saved, &id);
        let reload = core
            .shared
            .gate
            .call(
                "config/reload",
                json!({"sessionId":id,"reloadRuntime":true}),
                None,
            )
            .await
            .unwrap()
            .result;
        assert_eq!(reload["status"], "applied", "{reload}");
        assert_eq!(reload["strippedHistoryImages"], 0);
        assert!(matches!(
            crate::model_check::account_models_unavailable_reason(
                &core.shared,
                &reload["runtime"]["config"]
            ),
            Some(crate::model_check::AccountModelsUnavailableReason::NotFetched)
        ));
        assert_eq!(reload["runtime"]["activeApprovalPolicy"]["name"], policy);
        eprintln!(
            "PINNED_315_OFFLINE_RELOAD policy={policy} status=applied account_models=not_fetched"
        );
        chats.push(id);
    }
    let file = root.join("sessions/model-choices.json");
    let before = std::fs::read(&file).unwrap();
    let mut events = core.subscribe(None);
    let (ui, debug) = tokio::join!(
        super::super::send::send_error(core, &chats[0], false),
        super::super::send::send_error(core, &chats[1], true),
    );
    for error in [ui, debug] {
        assert_eq!(error.code, crate::ErrorCode::InvalidParams, "{error}");
        assert_eq!(error.message, super::super::send::NOT_FETCHED);
    }
    assert_eq!(
        std::fs::read(file).unwrap(),
        before,
        "refresh/Send changed saved picks"
    );
    assert_eq!(super::check(core).await["gone"], Value::Null);
    assert!(
        !crate::refresh_notice_tests::drain_events(&mut events)
            .await
            .iter()
            .any(|event| matches!(
                event,
                crate::CoreEvent::Alert {
                    kind: crate::AlertKind::ModelChoiceNotRestored,
                    ..
                }
            ))
    );
    eprintln!(
        "PINNED_315_OFFLINE_AFTER_REFRESH=PASS policy={policy} routes=UI,DebugApi concurrent_chats=2 launch=undecided picks=unchanged notice=none"
    );
}

async fn rpc(core: &crate::Core, method: &str, id: &str) -> Value {
    core.shared
        .gate
        .call(method, json!({"sessionId":id}), None)
        .await
        .unwrap()
        .result
}

pub(super) fn supervisor(
    root: &Path,
    home: PathBuf,
    closed: std::net::SocketAddr,
    delayed: Option<&DelayedCatalog>,
) -> vw_supervisor::SupervisorConfig {
    let python = std::env::var_os("VIBE_PYTHON").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vibe-3.1.5/venv/bin/python"),
        PathBuf::from,
    );
    let mut supervisor = vw_supervisor::SupervisorConfig::new(
        python.with_file_name("vibe-app-server"),
        home,
        root.to_path_buf(),
    );
    supervisor.home = Some(root.join("home"));
    std::fs::create_dir_all(supervisor.home.as_ref().unwrap()).unwrap();
    supervisor.parent_env = Some(vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
    ]);
    supervisor.extra_env = vec![
        (
            "VIBE_CONSOLE_BASE_URL".into(),
            format!("http://{closed}/console").into(),
        ),
        (
            "VIBE_VIBE_BASE_URL".into(),
            format!("http://{closed}/chat").into(),
        ),
        (
            "PYTHON_KEYRING_BACKEND".into(),
            "keyring.backends.null.Keyring".into(),
        ),
    ];
    if let Some(catalog) = delayed {
        // A stalled catalog outlives the host read cap; it is a separate ordering
        // case from the closed-port cases above, which have no provider override.
        supervisor.extra_env.push((
            "VIBE_PROVIDERS".into(),
            json!([{"name":"mistral","api_base":format!("{}/v1", catalog.base),
                "api_key_env_var":"MISTRAL_API_KEY","backend":"mistral"}])
            .to_string()
            .into(),
        ));
    }
    supervisor
}
