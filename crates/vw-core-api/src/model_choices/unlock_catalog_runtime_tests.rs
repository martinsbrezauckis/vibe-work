//! A real pinned Vibe catalog session seeded as a 0.1.18 saved-choice record.
//! The parent fixture supplies a local catalog and model endpoint; these checks exercise Keys-page unlock and Send.
use super::*;

fn catalog_config(root: &Path) {
    std::fs::write(
        root.join("vibe-home/config.toml"),
        "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
         enable_model_catalog = true\nactive_model = 'mistral-large-4-0'\n",
    )
    .unwrap();
}

fn runtime_summary(result: &Value) -> Value {
    let runtime = &result["runtime"];
    json!({
        "ready": result["ready"],
        "activeModel": runtime["config"]["activeModel"],
        "models": runtime["config"]["models"].as_array().map(|rows| rows
            .iter().map(|row| row["alias"].clone()).collect::<Vec<_>>()),
        "validationWarnings": runtime["config"]["validationWarnings"],
        "modelChoiceRestore": result["modelChoiceRestore"],
    })
}

fn trace_saved_state(core: &crate::Core, root: &Path, id: &str, phase: &str) {
    let stored = core.shared.store.runtime(id);
    eprintln!(
        "CATALOG_UNLOCK_TRACE {phase} stored_active={} saved={}",
        stored
            .as_ref()
            .map_or(Value::Null, |runtime| runtime["config"]["activeModel"]
                .clone()),
        saved(root, id),
    );
}

async fn seeded_chat(
    core: &crate::Core,
    api: &LocalModelApi,
    work: &Path,
    model: &str,
    effort: &str,
) -> String {
    let started = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartSession {
                agent_config: Some(json!({"cwd":work,"approvalPolicy":"accept-edits",
                    "trustWorkspace":true})),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = started else {
        panic!("expected a new session")
    };
    let id = snapshot.session_id;
    wait_for_offer(core, &id, api).await;
    core.execute(
        crate::Origin::Ui,
        crate::Command::PickModel {
            model_name: None,
            model_display_name: None,
            session_id: id.clone(),
            model_alias: Some(model.into()),
            reasoning_effort: Some(effort.into()),
        },
    )
    .await
    .unwrap();
    core.execute(
        crate::Origin::Ui,
        crate::Command::StartTurn {
            session_id: id.clone(),
            message: vec![json!({"type":"text","text":"Seed the 0.1.18 chat."})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
    let request = api.wait_for(model).await;
    assert_eq!(request["model"], model, "{request}");
    wait_completed(core, &id).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if saved(work.parent().unwrap(), &id)["pending"].is_null() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        saved(work.parent().unwrap(), &id)["remembered"]["effort"],
        effort
    );
    id
}

fn seed_018_record(root: &Path, first_id: &str, second_id: &str, pending: bool) -> Value {
    let file = root.join("sessions/model-choices.json");
    let mut document: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    // The 0.1.18 version-1 file has both backend-bound record forms: pending
    // plus remembered for its first chat, remembered alone for its second.
    let remembered = document["chats"][first_id]["remembered"].clone();
    assert_eq!(remembered["alias"], SAVED);
    assert!(
        remembered["provider_key"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    );
    if pending {
        document["chats"][first_id]["pending"] = remembered;
    } else {
        assert!(document["chats"][first_id]["pending"].is_null());
    }
    assert!(document["chats"][second_id]["pending"].is_null());
    assert!(
        document["chats"][second_id]["remembered"]["provider_key"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
    );
    std::fs::write(&file, serde_json::to_vec(&document).unwrap()).unwrap();
    document
}

#[allow(clippy::too_many_lines)] // Two seeded chats cross the same real locked-start and typed unlock boundary.
async fn run_catalog_unlock_case(
    effort: &str,
    pending: bool,
    first_origin: crate::Origin,
    checked_read: bool,
) {
    let api = LocalModelApi::start_with_catalog(true).await;
    let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    catalog_config(root.path());
    let keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let first = seeded_chat(&core, &api, &work, SAVED, effort).await;
    let second = seeded_chat(&core, &api, &work, "mistral-large-4-0", "high").await;
    keys.lock();
    core.shutdown().await;
    drop(keys);
    let seeded = seed_018_record(root.path(), &first, &second, pending);
    api.requests.lock().unwrap().clear();

    let keys = restarted_keys(root.path(), &api).await;
    assert!(!keys.speech_available());
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    for id in [&first, &second] {
        let resumed = core
            .execute(
                crate::Origin::Ui,
                crate::Command::ResumeSession {
                    session_id: id.clone(),
                    agent_config: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(resumed, crate::CommandResult::Session { .. }));
        let settled = call(&core, "session/ready/wait", json!({"sessionId":id})).await;
        assert_eq!(settled["ready"], true, "{settled}");
    }
    let locked_runtime = call(&core, "runtime/read", json!({"sessionId":first})).await;
    assert_eq!(locked_runtime["ready"], true, "{locked_runtime}");
    assert!(
        locked_runtime["runtime"]["config"]["models"]
            .as_array()
            .is_none_or(|rows| rows.iter().all(|row| row["alias"] != SAVED)),
        "a locked catalog unexpectedly offered the saved account model: {locked_runtime}"
    );
    assert_eq!(saved(root.path(), &first), seeded["chats"][&first]);
    assert_eq!(saved(root.path(), &second), seeded["chats"][&second]);

    // The Keys page sends this typed command. No manual config/reload or picker
    // action occurs between unlock and the first read or Send.
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
    assert!(keys.speech_available());
    let mut events = core.subscribe(Some(first.clone()));
    trace_saved_state(&core, root.path(), &first, "before_send");
    if checked_read {
        // The notice controller reads this once after the key becomes usable.
        // The reply must name the restored chat model, not the locked fallback.
        let checked = call(&core, "runtime/read", json!({"sessionId":first})).await;
        let config = &checked["runtime"]["config"];
        let summary = runtime_summary(&checked);
        assert_eq!(config["activeModel"]["alias"], SAVED, "{summary}");
        assert_eq!(config["activeModel"]["thinking"], effort, "{summary}");
        assert!(
            config["models"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|row| row["alias"] == SAVED)),
            "the unlocked chat has no saved catalog row: {summary}"
        );
        assert_eq!(checked["modelChoiceRestore"], "checked", "{summary}");
    }

    // UI composer and Debug API both use this typed StartTurn entrypoint.
    // A GLM saved pick is exercised through each origin across the two cases.
    core.execute(
        first_origin,
        crate::Command::StartTurn {
            session_id: first.clone(),
            message: vec![json!({"type":"text","text":"After unlock, first."})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
    trace_saved_state(&core, root.path(), &first, "after_start_turn");
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(30), events.next()).await
    {
        if let crate::CoreEvent::Alert { kind, message, .. } = event {
            eprintln!("CATALOG_UNLOCK_TRACE alert kind={kind:?} message={message}");
        }
    }
    let first_request = api.wait_for(SAVED).await;
    assert_eq!(first_request["model"], SAVED, "{first_request}");
    if effort == "off" {
        assert!(
            first_request["reasoning_effort"].is_null()
                || first_request["reasoning_effort"] == "none",
            "Off became another wire reasoning level: {first_request}"
        );
    } else {
        assert_eq!(first_request["reasoning_effort"], effort, "{first_request}");
    }
    wait_completed(&core, &first).await;
    let first_run = call(&core, "runtime/read", json!({"sessionId":first})).await;
    assert_eq!(
        first_run["runtime"]["config"]["activeModel"]["alias"],
        SAVED
    );
    assert_eq!(
        first_run["runtime"]["config"]["activeModel"]["thinking"],
        effort
    );
    assert!(
        first_run["runtime"]["config"]["validationWarnings"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{first_run}"
    );

    core.execute(
        if first_origin == crate::Origin::Ui {
            crate::Origin::DebugApi
        } else {
            crate::Origin::Ui
        },
        crate::Command::StartTurn {
            session_id: second.clone(),
            message: vec![json!({"type":"text","text":"After unlock, second."})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
    let second_request = api.wait_for("mistral-large-4-0").await;
    assert_eq!(second_request["model"], "mistral-large-4-0");
    assert_eq!(second_request["reasoning_effort"], "high");
    wait_completed(&core, &second).await;
    let second_run = call(&core, "runtime/read", json!({"sessionId":second})).await;
    assert_eq!(
        second_run["runtime"]["config"]["activeModel"]["alias"],
        "mistral-large-4-0"
    );
    assert_eq!(
        second_run["runtime"]["config"]["activeModel"]["thinking"],
        "high"
    );
    let first_saved = saved(root.path(), &first);
    let second_saved = saved(root.path(), &second);
    assert_eq!(first_saved["remembered"]["alias"], SAVED);
    assert_eq!(first_saved["remembered"]["effort"], effort);
    assert_eq!(
        first_saved["remembered"]["provider_key"],
        seeded["chats"][&first]["remembered"]["provider_key"]
    );
    assert!(first_saved["pending"].is_null(), "{first_saved}");
    assert_eq!(second_saved["remembered"]["alias"], "mistral-large-4-0");
    assert_eq!(second_saved["remembered"]["effort"], "high");
    assert_eq!(
        second_saved["remembered"]["provider_key"],
        seeded["chats"][&second]["remembered"]["provider_key"]
    );
    assert!(
        api.routes
            .lock()
            .unwrap()
            .iter()
            .any(|route| { route.starts_with("GET /v1/model-catalog?") })
    );
    eprintln!(
        "PINNED_315_CATALOG_UNLOCK effort={effort} pending={pending} origin={first_origin:?}"
    );
    core.shutdown().await;
    api.task.abort();
}

#[tokio::test]
async fn pinned_315_catalog_unlock_replays_pending_low_and_remembered_high() {
    run_catalog_unlock_case("low", true, crate::Origin::Ui, false).await;
}

#[tokio::test]
async fn pinned_315_catalog_unlock_replays_remembered_off_through_debug_api() {
    run_catalog_unlock_case("off", false, crate::Origin::DebugApi, false).await;
}

#[tokio::test]
async fn pinned_315_catalog_unlock_read_checks_choice_before_notice_and_send() {
    run_catalog_unlock_case("low", true, crate::Origin::Ui, true).await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One fixture records the same pinned default across three Vibe starts.
async fn defaultpin_closed_catalog_then_networked_start_preserves_glm() {
    let api = LocalModelApi::start_with_catalog(true).await;
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let config = root.path().join("vibe-home/config.toml");
    std::fs::write(
        &config,
        "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
         enable_model_catalog = true\nactive_model = 'zai-glm-5-3-flash'\n",
    )
    .unwrap();
    let original_config = std::fs::read_to_string(&config).unwrap();
    let keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    let before = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN B initial={} toml={}",
        json!({
        "pin":before["config"]["activeModelPinned"],
        "active":before["config"]["activeModel"],
        "default":before["config"]["defaultModelAlias"]}),
        std::fs::read_to_string(&config).unwrap()
    );
    assert_eq!(before["config"]["activeModelPinned"], true, "{before}");
    assert_eq!(
        before["config"]["activeModel"]["alias"],
        "zai-glm-5-3-flash"
    );
    core.shutdown().await;
    let endpoint = api.base.trim_start_matches("http://").to_owned();
    api.task.abort();
    let _ = api.task.await;

    let core = keyed_core(root.path(), &work, &keys).await;
    let offline = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN B offline={} toml={}",
        json!({
        "pin":offline["config"]["activeModelPinned"],
        "active":offline["config"]["activeModel"],
        "default":offline["config"]["defaultModelAlias"]}),
        std::fs::read_to_string(&config).unwrap()
    );
    // Vibe cannot resolve the saved alias while the catalog is absent. Core must
    // preserve its identity for the composer instead of adopting Large 4.
    assert_eq!(offline["config"]["activeModelPinned"], false, "{offline}");
    assert_eq!(
        offline["savedDefaultUnresolved"]["alias"], "zai-glm-5-3-flash",
        "{offline}"
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original_config);
    core.register_picked_folder(&work).unwrap();
    let started = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartSession {
                agent_config: Some(
                    json!({"cwd":work,"approvalPolicy":"ask","trustWorkspace":true}),
                ),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = started else {
        panic!("expected new chat")
    };
    let offline_id = snapshot.session_id;
    let blocked = core
        .execute(
            crate::Origin::Ui,
            crate::Command::StartTurn {
                session_id: offline_id.clone(),
                message: vec![json!({"type":"text","text":"No silent Large 4 fallback."})],
                client_user_message_id: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        blocked.message.contains("could not reach Mistral"),
        "{blocked:?}"
    );
    let api = LocalModelApi::start_with_catalog_on(true, &endpoint).await;
    let same_process = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN B port_reopened_same_process={} routes={:?} toml={}",
        json!({
            "pin":same_process["config"]["activeModelPinned"],
            "active":same_process["config"]["activeModel"],
            "default":same_process["config"]["defaultModelAlias"]
        }),
        api.routes.lock().unwrap(),
        std::fs::read_to_string(&config).unwrap()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original_config);
    if same_process["config"]["activeModelPinned"] == false {
        assert_eq!(
            same_process["savedDefaultUnresolved"]["alias"], "zai-glm-5-3-flash",
            "{same_process}"
        );
    }
    let refreshed = call(&core, "config/read", json!({"refreshModelCatalog":true})).await;
    eprintln!(
        "DEFAULTPIN B refreshed_same_process={} routes={:?}",
        json!({
            "pin":refreshed["config"]["activeModelPinned"],
            "active":refreshed["config"]["activeModel"],
            "default":refreshed["config"]["defaultModelAlias"]
        }),
        api.routes.lock().unwrap()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original_config);
    let reload = core
        .shared
        .gate
        .call(
            "config/reload",
            json!({"sessionId":offline_id,"reloadRuntime":true}),
            None,
        )
        .await;
    eprintln!(
        "DEFAULTPIN B attached_reload={:?} routes={:?}",
        reload
            .as_ref()
            .map(|reply| json!({
                "status":reply.result["status"],
                "active":reply.result["runtime"]["config"]["activeModel"],
                "pin":reply.result["runtime"]["config"]["activeModelPinned"]
            }))
            .map_err(ToString::to_string),
        api.routes.lock().unwrap()
    );
    let after_reload = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN B lobby_after_attached_reload={}",
        json!({
            "pin":after_reload["config"]["activeModelPinned"],
            "active":after_reload["config"]["activeModel"],
            "saved":after_reload["savedDefaultUnresolved"]
        })
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original_config);
    core.shutdown().await;
    drop(keys);

    let keys = restarted_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
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
    assert!(keys.speech_available());
    let mut networked = call(&core, "config/read", json!({})).await;
    for _ in 0..30 {
        if networked["config"]["activeModelPinned"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        networked = call(&core, "config/read", json!({})).await;
    }
    eprintln!(
        "DEFAULTPIN B networked={} routes={:?} toml={}",
        json!({
        "pin":networked["config"]["activeModelPinned"],
        "active":networked["config"]["activeModel"],
        "default":networked["config"]["defaultModelAlias"]}),
        api.routes.lock().unwrap(),
        std::fs::read_to_string(&config).unwrap()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), original_config);
    assert_eq!(
        networked["savedDefaultAlias"],
        if networked["config"]["activeModelPinned"] == true {
            Value::Null
        } else {
            json!("zai-glm-5-3-flash")
        },
        "{networked}"
    );
    if networked["config"]["activeModelPinned"] == false {
        assert_eq!(
            networked["savedDefaultUnresolved"]["alias"], "zai-glm-5-3-flash",
            "{networked}"
        );
    }
    core.shutdown().await;
    api.task.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The unavailable saved choice crosses quit, unlock, first Send and notice.
async fn defaultpin_unavailable_chat_fallback_preserves_glm_default() {
    let api = LocalModelApi::start_with_catalog(true).await;
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let config = root.path().join("vibe-home/config.toml");
    std::fs::write(
        &config,
        "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
         enable_model_catalog = true\nactive_model = 'zai-glm-5-3-flash'\n",
    )
    .unwrap();
    let keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let id = seeded_chat(&core, &api, &work, OTHER, "high").await;
    let before = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN A initial={} toml={}",
        json!({
        "pin":before["config"]["activeModelPinned"],
        "active":before["config"]["activeModel"],
        "default":before["config"]["defaultModelAlias"]}),
        std::fs::read_to_string(&config).unwrap()
    );
    assert_eq!(before["config"]["activeModelPinned"], true, "{before}");
    core.shutdown().await;
    keys.lock();
    drop(keys);
    let record_file = root.path().join("sessions/model-choices.json");
    let mut records: Value = serde_json::from_slice(&std::fs::read(&record_file).unwrap()).unwrap();
    records["chats"][&id]["remembered"]["alias"] = json!("vw-test-unavailable-model");
    records["chats"][&id]["remembered"]["name"] = json!("vw-test-unavailable-model");
    std::fs::write(&record_file, serde_json::to_vec(&records).unwrap()).unwrap();

    let keys = restarted_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let mut events = core.subscribe(Some(id.clone()));
    let resumed = core
        .execute(
            crate::Origin::Ui,
            crate::Command::ResumeSession {
                session_id: id.clone(),
                agent_config: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(resumed, crate::CommandResult::Session { .. }));
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
    core.execute(
        crate::Origin::Ui,
        crate::Command::StartTurn {
            session_id: id.clone(),
            message: vec![json!({"type":"text","text":"Fallback once."})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
    let request = api.wait_for("zai-glm-5-3-flash").await;
    assert_eq!(request["model"], "zai-glm-5-3-flash", "{request}");
    wait_completed(&core, &id).await;
    let after = call(&core, "config/read", json!({})).await;
    eprintln!(
        "DEFAULTPIN A after={} toml={}",
        json!({
        "pin":after["config"]["activeModelPinned"],
        "active":after["config"]["activeModel"],
        "default":after["config"]["defaultModelAlias"]}),
        std::fs::read_to_string(&config).unwrap()
    );
    assert_eq!(after["config"]["activeModelPinned"], true, "{after}");
    assert_eq!(after["config"]["activeModel"]["alias"], "zai-glm-5-3-flash");
    let mut notices = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), events.next()).await
    {
        if let crate::CoreEvent::Alert {
            kind: crate::AlertKind::ModelChoiceNotRestored,
            message,
            ..
        } = event
        {
            notices.push(message);
        }
    }
    assert_eq!(notices.len(), 1, "{notices:?}");
    core.shutdown().await;
    api.task.abort();
}
