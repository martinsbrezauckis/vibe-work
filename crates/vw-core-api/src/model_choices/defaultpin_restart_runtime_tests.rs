//! Real pinned Vibe recovery of the saved new-chat default after a vault unlock.
//! A Vibe started without a usable catalog keeps reporting the saved default unpinned in the same process; Core asks
//! the supervisor for one fresh process after the unlock when no chat is busy. The parent fixture supplies the
//! isolated key service and the local catalog and model endpoint; no real credential is used.
use super::*;

const GLM: &str = "zai-glm-5-3-flash";

fn glm_config(root: &Path, catalog: bool) -> PathBuf {
    let config = root.join("vibe-home/config.toml");
    std::fs::write(
        &config,
        format!(
            "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
             enable_model_catalog = {catalog}\nactive_model = '{GLM}'\n"
        ),
    )
    .unwrap();
    config
}

fn generation(core: &crate::Core) -> Option<u64> {
    match &*core.shared.supervisor.state().borrow() {
        vw_supervisor::SupervisorState::Ready { generation, .. } => Some(*generation),
        _ => None,
    }
}

fn lobby_summary(read: &Value) -> Value {
    json!({
        "pin": read["config"]["activeModelPinned"],
        "active": read["config"]["activeModel"]["alias"],
        "thinking": read["config"]["activeModel"]["thinking"],
        "default": read["config"]["defaultModelAlias"],
        "saved": read["savedDefaultUnresolved"],
    })
}

async fn unlock(core: &crate::Core) {
    let unlocked = core
        .execute(
            crate::Origin::Ui,
            crate::Command::Keys {
                request: crate::KeyRequest::UnlockVault {
                    passphrase: crate::SecretInput::new("unlockpick test password"),
                    vault: None,
                    unlock_method: None,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(unlocked, crate::CommandResult::Done {}));
}

async fn new_chat(core: &crate::Core, work: &Path, policy: &str) -> String {
    let started = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartSession {
                agent_config: Some(
                    json!({"cwd":work,"approvalPolicy":policy,"trustWorkspace":true}),
                ),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = started else {
        panic!("expected a new chat")
    };
    snapshot.session_id
}

async fn send(core: &crate::Core, id: &str, text: &str) {
    core.execute(
        crate::Origin::Ui,
        crate::Command::StartTurn {
            session_id: id.to_owned(),
            message: vec![json!({"type":"text","text":text})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One fixture crosses a locked start, a typed unlock, the restart and a new chat.
async fn defaultpin_locked_start_unlock_restarts_once_and_new_chat_runs_saved_glm_low() {
    let api = LocalModelApi::start_with_catalog(true).await;
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let config = glm_config(root.path(), true);
    let original = std::fs::read_to_string(&config).unwrap();
    // A vault with a key, then a fresh key service: the next start is locked, like "password each time".
    drop(isolated_keys(root.path(), &api).await);
    let keys = restarted_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    // The lobby reads its config before the person unlocks (the app reads it at start).
    let locked = call(&core, "config/read", json!({})).await;
    eprintln!("DEFAULTPIN R locked={}", lobby_summary(&locked));
    assert_eq!(locked["config"]["activeModelPinned"], false, "{locked}");
    assert_eq!(locked["savedDefaultUnresolved"]["alias"], GLM, "{locked}");
    assert_eq!(generation(&core), Some(1));

    let unlock_started = std::time::Instant::now();
    unlock(&core).await;
    let unlock_ms = unlock_started.elapsed().as_millis();
    let unlocked_at = std::time::Instant::now();
    // The unlock answers once the restarted server is ready; a read still meeting the restart is retried.
    let mut lobby = Value::Null;
    for _ in 0..150 {
        let read = core
            .execute(
                crate::Origin::Ui,
                crate::Command::Call {
                    method: "config/read".into(),
                    params: json!({}),
                    confirmation_token: None,
                },
            )
            .await;
        if let Ok(crate::CommandResult::Reply { result, .. }) = read {
            lobby = result;
            if lobby["config"]["activeModelPinned"] == true {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!("DEFAULTPIN R unlock_ms={unlock_ms}");
    eprintln!(
        "DEFAULTPIN R after_unlock={} after_ms={} generation={:?} routes={:?}",
        lobby_summary(&lobby),
        unlocked_at.elapsed().as_millis(),
        generation(&core),
        api.routes.lock().unwrap()
    );
    assert_eq!(lobby["config"]["activeModelPinned"], true, "{lobby}");
    assert_eq!(lobby["config"]["activeModel"]["alias"], GLM, "{lobby}");
    assert!(lobby["savedDefaultUnresolved"].is_null(), "{lobby}");
    assert_eq!(generation(&core), Some(2), "exactly one restart");
    // At most once per unlock: nothing asks again later.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(generation(&core), Some(2), "no second restart");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);

    // New chat, implicit model: the composer applies the saved thinking preference (`composerDefaults.ts`) as a
    // thinking-only pick before the first message; the model stays Vibe's pinned default.
    let id = new_chat(&core, &work, "accept-edits").await;
    let runtime = call(&core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(
        runtime["runtime"]["config"]["activeModel"]["alias"], GLM,
        "{runtime}"
    );
    core.execute(
        crate::Origin::Ui,
        crate::Command::PickModel {
            model_name: None,
            model_display_name: None,
            session_id: id.clone(),
            model_alias: None,
            reasoning_effort: Some("low".into()),
        },
    )
    .await
    .unwrap();
    send(&core, &id, "New chat after unlock.").await;
    let request = api.wait_for(GLM).await;
    eprintln!(
        "DEFAULTPIN R first_send model={} effort={}",
        request["model"], request["reasoning_effort"]
    );
    assert_eq!(request["model"], GLM, "{request}");
    assert_eq!(request["reasoning_effort"], "low", "{request}");
    wait_completed(&core, &id).await;
    assert_eq!(generation(&core), Some(2));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
    core.shutdown().await;
    api.task.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One fixture holds an approval card across a lock and unlock.
async fn defaultpin_unlock_with_open_approval_keeps_guard_and_does_not_restart() {
    // No catalog rows: Vibe cannot resolve the saved GLM and reports it unpinned, while the local
    // Large 4 row still answers turns through the unlocked key.
    let api = LocalModelApi::start_with_catalog(false).await;
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let config = glm_config(root.path(), true);
    let original = std::fs::read_to_string(&config).unwrap();
    let keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let before = call(&core, "config/read", json!({})).await;
    eprintln!("DEFAULTPIN R busy_before={}", lobby_summary(&before));
    assert_eq!(before["config"]["activeModelPinned"], false, "{before}");
    assert_eq!(before["savedDefaultUnresolved"]["alias"], GLM, "{before}");

    let id = new_chat(&core, &work, "ask").await;
    // An explicit pick of the local row: the unreachable catalog cannot check the saved default.
    core.execute(
        crate::Origin::Ui,
        crate::Command::PickModel {
            model_name: None,
            model_display_name: None,
            session_id: id.clone(),
            model_alias: Some("mistral-large-4-0".into()),
            reasoning_effort: None,
        },
    )
    .await
    .unwrap();
    send(&core, &id, "defaultpin-approval: write the file.").await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while !crate::recovery::working(&core.shared, &id) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the chat shows an approval card");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(crate::recovery::working(&core.shared, &id));
    let view = core.shared.store.view(&id).unwrap();
    eprintln!(
        "DEFAULTPIN R busy_state status={} callbacks={}",
        view.state
            .pointer("/session/status")
            .cloned()
            .unwrap_or(Value::Null),
        view.state
            .pointer("/activeCallbacks")
            .cloned()
            .unwrap_or(Value::Null)
    );

    keys.lock();
    tokio::time::sleep(Duration::from_millis(300)).await;
    unlock(&core).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let after = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN R busy_after={} generation={:?} working={}",
        lobby_summary(&after),
        generation(&core),
        crate::recovery::working(&core.shared, &id)
    );
    assert_eq!(
        generation(&core),
        Some(1),
        "no restart while a chat is busy"
    );
    assert!(crate::recovery::working(&core.shared, &id));
    // The guard stays: Core still marks the saved default as unconfirmed.
    assert_eq!(after["config"]["activeModelPinned"], false, "{after}");
    assert_eq!(after["savedDefaultUnresolved"]["alias"], GLM, "{after}");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
    core.shutdown().await;
    api.task.abort();
}
