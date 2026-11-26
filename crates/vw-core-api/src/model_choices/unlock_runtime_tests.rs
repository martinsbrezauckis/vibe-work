//! Real pinned Vibe unlock and Send regression with a controlled local model list and provider endpoint.
//! The parent module supplies the isolated Vibe launch; the key service and HTTP fixture hold no real credentials.
use super::*;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SAVED: &str = "unlockpick-local-saved";
const OTHER: &str = "unlockpick-local-other";
const TEST_KEY: &str = "unlockpick-test-key-0000000000000000";

#[path = "defaultpin_restart_runtime_tests.rs"]
mod defaultpin_restart_runtime_tests;
#[path = "unlock_catalog_runtime_tests.rs"]
mod unlock_catalog_runtime_tests;

fn local_completion(request: &Value) -> (&'static str, &'static str, String) {
    let model = request["model"].as_str().unwrap_or(SAVED);
    let last = request["messages"]
        .as_array()
        .and_then(|all| all.last())
        .cloned();
    // A marked first message asks for a file write, so an Ask chat waits on an approval card.
    let approval =
        last.is_some_and(|m| m["role"] == "user" && m.to_string().contains("defaultpin-approval"));
    let delta = if approval {
        json!({"role":"assistant","content":"","tool_calls":[{"id":"call_dp1","type":"function","index":0,
            "function":{"name":"write_file","arguments":"{\"path\":\"dp.txt\",\"content\":\"x\"}"}}]})
    } else {
        json!({"role":"assistant","content":"OK"})
    };
    if request["stream"] == true {
        let chunk = json!({"id":"unlockpick-local","object":"chat.completion.chunk","created":1,
            "model":model,"choices":[{"index":0,"delta":delta,"finish_reason":if approval {"tool_calls"} else {"stop"}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        (
            "200 OK",
            "text/event-stream",
            format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        )
    } else {
        let answer = json!({"id":"unlockpick-local","object":"chat.completion","created":1,
            "model":model,"choices":[{"index":0,"message":{"role":"assistant","content":"OK"},
            "finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        ("200 OK", "application/json", answer.to_string())
    }
}

fn local_catalog() -> (&'static str, &'static str, String) {
    let models = [SAVED, OTHER, "mistral-large-4-0", "zai-glm-5-3-flash"].map(|id| {
        json!({"id":id,"label":id,"provider":"mistral",
            "max_context_length":32768,
            "capabilities":{"completion_chat":true,"function_calling":true},
            "parameters":{"reasoning":["none","low","high"]}})
    });
    (
        "200 OK",
        "application/json",
        json!({"models":models}).to_string(),
    )
}

struct LocalModelApi {
    base: String,
    requests: Arc<Mutex<Vec<Value>>>,
    routes: Arc<Mutex<Vec<String>>>,
    authorized_saved: Arc<AtomicU64>,
    authorized_other: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl LocalModelApi {
    async fn start() -> Self {
        Self::start_with_catalog(false).await
    }

    async fn start_with_catalog(catalog: bool) -> Self {
        Self::start_with_catalog_on(catalog, "127.0.0.1:0").await
    }

    async fn start_with_catalog_on(catalog: bool, bind: &str) -> Self {
        let listener = TcpListener::bind(bind).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(Mutex::new(Vec::new()));
        let authorized_saved = Arc::new(AtomicU64::new(0));
        let authorized_other = Arc::new(AtomicU64::new(0));
        let captured = Arc::clone(&requests);
        let seen_routes = Arc::clone(&routes);
        let saved_auth = Arc::clone(&authorized_saved);
        let other_auth = Arc::clone(&authorized_other);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let captured = Arc::clone(&captured);
                let seen_routes = Arc::clone(&seen_routes);
                let saved_auth = Arc::clone(&saved_auth);
                let other_auth = Arc::clone(&other_auth);
                tokio::spawn(async move {
                    let mut input = Vec::new();
                    let mut buffer = [0_u8; 8192];
                    let header = loop {
                        let Ok(n) = stream.read(&mut buffer).await else {
                            return;
                        };
                        if n == 0 || input.len() + n > 262_144 {
                            return;
                        }
                        input.extend_from_slice(&buffer[..n]);
                        if let Some(end) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&input[..header]).into_owned();
                    let route = headers.lines().next().unwrap_or_default();
                    let method_path = route
                        .split_whitespace()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" ");
                    seen_routes.lock().unwrap().push(method_path);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    while input.len() < header + length {
                        let Ok(n) = stream.read(&mut buffer).await else {
                            return;
                        };
                        if n == 0 || input.len() + n > 262_144 {
                            return;
                        }
                        input.extend_from_slice(&buffer[..n]);
                    }
                    let (status, content_type, body) = if route.starts_with("POST ") {
                        // Retain only the match, never a credential header or its value.
                        let has_test_key = headers.lines().any(|line| {
                            line.trim()
                                .eq_ignore_ascii_case(&format!("authorization: Bearer {TEST_KEY}"))
                        });
                        if let Ok(body) =
                            serde_json::from_slice::<Value>(&input[header..header + length])
                        {
                            if has_test_key && body["model"] == SAVED {
                                saved_auth.fetch_add(1, Ordering::Relaxed);
                            }
                            if has_test_key && body["model"] == OTHER {
                                other_auth.fetch_add(1, Ordering::Relaxed);
                            }
                            let response = local_completion(&body);
                            captured.lock().unwrap().push(body);
                            response
                        } else {
                            ("400 Bad Request", "application/json", "{}".to_owned())
                        }
                    } else if catalog && route.starts_with("GET /v1/model-catalog?") {
                        local_catalog()
                    } else {
                        ("200 OK", "application/json", json!({"data":[]}).to_string())
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        Self {
            base,
            requests,
            routes,
            authorized_saved,
            authorized_other,
            task,
        }
    }

    async fn wait_for_model_request(&self) -> Value {
        self.wait_for(SAVED).await
    }

    async fn wait_for(&self, model: &str) -> Value {
        let answer = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(request) = self
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|request| request["model"] == model)
                    .cloned()
                {
                    return request;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        answer.unwrap_or_else(|_| {
            let observed: Vec<(Value, Value)> = self
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| {
                    (
                        request["model"].clone(),
                        request["reasoning_effort"].clone(),
                    )
                })
                .collect();
            panic!("no local request for model {model}; observed model/effort pairs: {observed:?}")
        })
    }
}

fn key_config(root: &Path, api: &LocalModelApi) -> vw_keys::KeyServiceConfig {
    std::fs::create_dir_all(root).unwrap();
    let proxy = vw_keyproxy::ProxyConfig {
        upstreams: vw_keyproxy::Upstreams {
            api: api.base.clone(),
            console: api.base.clone(),
            chat: api.base.clone(),
        },
        ..vw_keyproxy::ProxyConfig::default()
    };
    vw_keys::KeyServiceConfig {
        managed_vault: None,
        vault: vw_vault::VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(root.join("vault")),
            device_store: Arc::new(vw_vault::remember::MemoryDeviceStore::default()),
            kdf: vw_vault::KdfParams::default(),
            device_name: "unlockpick test".into(),
        },
        proxy,
        test_api_base: api.base.clone(),
        settings_path: root.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: root.join("missing.env"),
            keyring: Arc::new(vw_keys::existing::MemoryVibeKeyring::default()),
        },
    }
}

async fn isolated_keys_with_state(
    root: &Path,
    api: &LocalModelApi,
    keyed: bool,
) -> Arc<vw_keys::KeyService> {
    let keys = vw_keys::KeyService::start(key_config(root, api))
        .await
        .unwrap();
    keys.create_vault(vw_vault::SecretString::new("unlockpick test password"))
        .await
        .unwrap();
    if keyed {
        keys.store_key(vw_vault::SecretString::new(TEST_KEY))
            .await
            .unwrap();
    }
    keys
}

async fn isolated_keys(root: &Path, api: &LocalModelApi) -> Arc<vw_keys::KeyService> {
    isolated_keys_with_state(root, api, true).await
}

async fn restarted_keys(root: &Path, api: &LocalModelApi) -> Arc<vw_keys::KeyService> {
    vw_keys::KeyService::start(key_config(root, api))
        .await
        .unwrap()
}

async fn keyed_core(root: &Path, work: &Path, keys: &Arc<vw_keys::KeyService>) -> crate::Core {
    let mut supervisor = core_config(root, work);
    supervisor.extra_env.extend(keys.vibe_env());
    Box::pin(crate::Core::start(
        supervisor,
        keys.secret_provider(),
        NoPrompt,
        crate::CoreConfig {
            keys: Some(Arc::clone(keys)),
            sessions_dir: Some(root.join("sessions")),
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            ..crate::CoreConfig::default()
        },
    ))
    .await
    .unwrap()
}

fn write_local_rows(root: &Path, include_choices: bool, full_access: bool) {
    let mut config = String::from(
        "enable_telemetry = false\nenable_otel = false\nenable_connectors = false\n\
         enable_model_catalog = false\nactive_model = 'mistral-large-4-0'\n",
    );
    if full_access {
        config.push_str("default_approval_policy = 'auto-approve'\n");
    }
    if include_choices {
        for alias in [SAVED, OTHER] {
            write!(
                config,
                "[[models]]\nalias = '{alias}'\nname = '{alias}'\nprovider = 'mistral'\n\
                 max_context_length = 32768\nthinking_levels = ['off', 'low', 'high']\n"
            )
            .unwrap();
        }
    }
    std::fs::write(root.join("vibe-home/config.toml"), config).unwrap();
}

async fn offered(core: &crate::Core, id: &str, alias: &str) -> bool {
    let runtime = call(core, "runtime/read", json!({"sessionId":id})).await;
    runtime["runtime"]["config"]["models"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["alias"] == alias))
}

async fn wait_for_offer(core: &crate::Core, id: &str, api: &LocalModelApi) {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if offered(core, id, SAVED).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    if result.is_err() {
        let runtime = call(core, "runtime/read", json!({"sessionId":id})).await;
        let models: Vec<(Value, Value)> = runtime["runtime"]["config"]["models"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| (row["alias"].clone(), row["source"].clone()))
            .collect();
        let server_ready = matches!(
            &*core.shared.supervisor.state().borrow(),
            vw_supervisor::SupervisorState::Ready { .. }
        );
        panic!(
            "configured model absent: server_ready={server_ready} models(alias,source)={models:?} local routes={:?} redacted_stderr={:?}",
            api.routes.lock().unwrap(),
            core.status().server.stderr_tail
        );
    }
}

fn saved(root: &Path, id: &str) -> Value {
    let file: Value =
        serde_json::from_slice(&std::fs::read(root.join("sessions/model-choices.json")).unwrap())
            .unwrap();
    file["chats"][id].clone()
}

fn saved_bytes(root: &Path) -> Vec<u8> {
    std::fs::read(root.join("sessions/model-choices.json")).unwrap()
}

async fn detached_view(core: &crate::Core, id: &str) -> Value {
    let viewed = core
        .execute(
            crate::Origin::Ui,
            crate::Command::ViewSession {
                session_id: Some(id.into()),
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = viewed else {
        panic!("detached view must read the stored session")
    };
    snapshot.state["session"].clone()
}

async fn wait_completed(core: &crate::Core, id: &str) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let state = call(
                core,
                "session/read",
                json!({"sessionId":id,"history":{"limit":1},"turns":{"limit":1}}),
            )
            .await;
            let latest = state["state"]["turns"]
                .as_array()
                .and_then(|turns| turns.first());
            match latest.and_then(|turn| turn["status"].as_str()) {
                Some("completed") => return,
                Some("failed" | "interrupted") => panic!("local turn did not complete: {latest:?}"),
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pinned_315_remembered_choice_runs_after_locked_cold_reopen_and_unlock() {
    for generic in [false, true] {
        locked_reopen_and_unlock(generic).await;
    }
}

#[allow(clippy::too_many_lines)] // Cold restart, unlock and a completed provider turn share one disposable setup.
async fn locked_reopen_and_unlock(generic: bool) {
    let api = LocalModelApi::start().await;
    let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    write_local_rows(root.path(), true, false);
    let keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let session = core
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
    let crate::CommandResult::Session { snapshot, .. } = session else {
        panic!("expected session")
    };
    let id = snapshot.session_id;
    let active = call(&core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(active["runtime"]["activeApprovalPolicy"]["name"], "ask");
    wait_for_offer(&core, &id, &api).await;
    core.execute(
        crate::Origin::Ui,
        crate::Command::PickModel {
            model_name: None,
            model_display_name: None,
            session_id: id.clone(),
            model_alias: Some(SAVED.into()),
            reasoning_effort: Some("low".into()),
        },
    )
    .await
    .unwrap();
    let initial = saved(root.path(), &id);
    assert_eq!(initial["remembered"]["alias"], SAVED);
    assert_eq!(initial["remembered"]["effort"], "low");
    assert!(
        initial["remembered"]["provider_key"]
            .as_str()
            .is_some_and(|key| !key.is_empty())
    );
    // A real turn confirms Vibe's session pin and clears pending. The local provider
    // returns a completed answer after capturing the request; no external model is called.
    core.execute(
        crate::Origin::Ui,
        crate::Command::StartTurn {
            session_id: id.clone(),
            message: vec![json!({"type":"text","text":"First local turn."})],
            client_user_message_id: None,
        },
    )
    .await
    .unwrap();
    let first = api.wait_for_model_request().await;
    assert_eq!(first["model"], SAVED, "{first}");
    assert_eq!(first["reasoning_effort"], "low", "{first}");
    assert!(api.authorized_saved.load(Ordering::Relaxed) >= 1);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if saved(root.path(), &id)["pending"].is_null() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    wait_completed(&core, &id).await;
    api.requests.lock().unwrap().clear();
    api.authorized_saved.store(0, Ordering::Relaxed);
    let expected = saved(root.path(), &id);
    let expected_bytes = saved_bytes(root.path());
    keys.lock();
    core.shutdown().await;
    drop(keys);
    write_local_rows(root.path(), false, false);
    let keys = restarted_keys(root.path(), &api).await;
    assert!(!keys.speech_available());
    let core = keyed_core(root.path(), &work, &keys).await;
    core.register_picked_folder(&work).unwrap();
    let detached_session = detached_view(&core, &id).await;
    let detached_runtime = call(&core, "runtime/read", json!({"sessionId":id})).await;
    let reopened = core
        .execute(
            crate::Origin::Ui,
            crate::Command::ResumeSession {
                session_id: id.clone(),
                agent_config: None,
            },
        )
        .await
        .unwrap();
    let crate::CommandResult::Session { snapshot, .. } = reopened else {
        panic!("expected resumed session")
    };
    assert_eq!(
        snapshot.state["session"]["model"], "mistral-large-4-0",
        "{snapshot:?}"
    );
    let active = call(&core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(active["runtime"]["activeApprovalPolicy"]["name"], "ask");
    assert!(
        !offered(&core, &id, SAVED).await,
        "locked cold reopen must lack the account row"
    );
    assert_eq!(saved(root.path(), &id), expected);
    assert_eq!(saved_bytes(root.path()), expected_bytes);
    keys.unlock(
        vw_vault::SecretString::new("unlockpick test password"),
        None,
    )
    .await
    .unwrap();
    write_local_rows(root.path(), true, false);
    call(
        &core,
        "config/reload",
        json!({"sessionId":id,"reloadRuntime":true}),
    )
    .await;
    wait_for_offer(&core, &id, &api).await;
    let (origin, command) = if generic {
        (
            crate::Origin::Ui,
            crate::Command::Call {
                method: "turn/start".into(),
                params: json!({"sessionId":id,"message":[{"type":"text","text":"Reply with OK."}]}),
                confirmation_token: None,
            },
        )
    } else {
        (
            crate::Origin::DebugApi,
            crate::Command::StartTurn {
                session_id: id.clone(),
                message: vec![json!({"type":"text","text":"Reply with OK."})],
                client_user_message_id: None,
            },
        )
    };
    core.execute(origin, command).await.unwrap();
    let request = api.wait_for_model_request().await;
    assert_eq!(request["model"], SAVED, "{request}");
    assert_eq!(request["reasoning_effort"], "low", "{request}");
    assert_eq!(detached_session["model"], SAVED, "{detached_session}");
    assert_eq!(
        detached_session["reasoningEffort"], "low",
        "{detached_session}"
    );
    assert_eq!(
        detached_runtime["runtime"]["config"]["activeModel"]["alias"], SAVED,
        "{detached_runtime}"
    );
    assert_eq!(
        detached_runtime["runtime"]["config"]["activeModel"]["thinking"], "low",
        "{detached_runtime}"
    );
    assert!(api.authorized_saved.load(Ordering::Relaxed) >= 1);
    wait_completed(&core, &id).await;
    let runtime = call(&core, "runtime/read", json!({"sessionId":id})).await;
    assert_eq!(
        runtime["runtime"]["config"]["activeModel"]["alias"], SAVED,
        "{runtime}"
    );
    assert_eq!(
        runtime["runtime"]["config"]["activeModel"]["thinking"], "low",
        "{runtime}"
    );
    assert_eq!(saved(root.path(), &id), expected);
    assert_eq!(saved_bytes(root.path()), expected_bytes);
    eprintln!(
        "PINNED_315_UNLOCKPICK generic={generic} alias={SAVED} effort=low request={}",
        request["model"]
    );
    core.shutdown().await;
    api.task.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Both chats share one real Vibe instance and one disposable key transition.
async fn pinned_315_two_saved_chats_wait_for_new_key_then_send_their_own_choices() {
    let api = LocalModelApi::start().await;
    let base = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(root.path().join("vibe-home")).unwrap();
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    write_local_rows(root.path(), true, true);
    let original_keys = isolated_keys(root.path(), &api).await;
    let core = keyed_core(root.path(), &work, &original_keys).await;
    core.register_picked_folder(&work).unwrap();
    let mut chats = Vec::new();
    for (model, effort, policy) in [
        (SAVED, "low", "accept-edits"),
        (OTHER, "high", "auto-approve"),
    ] {
        let agent_config = if policy == "auto-approve" {
            json!({"cwd":work,"trustWorkspace":true})
        } else {
            json!({"cwd":work,"approvalPolicy":policy,"trustWorkspace":true})
        };
        let session = core
            .execute(
                crate::Origin::Ui,
                crate::Command::StartSession {
                    agent_config: Some(agent_config),
                },
            )
            .await
            .unwrap();
        let crate::CommandResult::Session { snapshot, .. } = session else {
            panic!("expected session")
        };
        let id = snapshot.session_id;
        let active = call(&core, "runtime/read", json!({"sessionId":id})).await;
        assert_eq!(active["runtime"]["activeApprovalPolicy"]["name"], policy);
        wait_for_offer(&core, &id, &api).await;
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
                message: vec![json!({"type":"text","text":"First local turn."})],
                client_user_message_id: None,
            },
        )
        .await
        .unwrap();
        let request = api.wait_for(model).await;
        assert_eq!(request["reasoning_effort"], effort, "{request}");
        wait_completed(&core, &id).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if saved(root.path(), &id)["pending"].is_null() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        chats.push((id, model, effort, policy));
    }
    let before: Vec<Value> = chats
        .iter()
        .map(|(id, _, _, _)| saved(root.path(), id))
        .collect();
    let before_bytes = saved_bytes(root.path());
    assert!(api.authorized_saved.load(Ordering::Relaxed) >= 1);
    assert!(api.authorized_other.load(Ordering::Relaxed) >= 1);
    core.shutdown().await;
    drop(original_keys);
    write_local_rows(root.path(), false, true);
    api.requests.lock().unwrap().clear();
    api.authorized_saved.store(0, Ordering::Relaxed);
    api.authorized_other.store(0, Ordering::Relaxed);

    // This vault is independent of the first and contains no credential until setup.
    let empty_keys = isolated_keys_with_state(&root.path().join("new-key"), &api, false).await;
    assert!(!empty_keys.speech_available());
    let core = keyed_core(root.path(), &work, &empty_keys).await;
    core.register_picked_folder(&work).unwrap();
    let mut detached = Vec::new();
    for (id, _, _, policy) in &chats {
        let view = detached_view(&core, id).await;
        let runtime = call(&core, "runtime/read", json!({"sessionId":id})).await;
        detached.push((view, runtime));
        let opened = core
            .execute(
                crate::Origin::Ui,
                crate::Command::ResumeSession {
                    session_id: id.clone(),
                    agent_config: None,
                },
            )
            .await
            .unwrap();
        let crate::CommandResult::Session { snapshot, .. } = opened else {
            panic!("expected session")
        };
        assert_eq!(
            snapshot.state["session"]["model"], "mistral-large-4-0",
            "{snapshot:?}"
        );
        let active = call(&core, "runtime/read", json!({"sessionId":id})).await;
        assert_eq!(active["runtime"]["activeApprovalPolicy"]["name"], *policy);
        let refusal = super::super::before(&core.shared, "turn/start", &json!({"sessionId":id}))
            .await
            .unwrap_err();
        assert!(refusal.message.contains("add a Mistral key"), "{refusal}");
    }
    assert!(api.requests.lock().unwrap().is_empty());
    empty_keys
        .store_key(vw_vault::SecretString::new(TEST_KEY))
        .await
        .unwrap();
    write_local_rows(root.path(), true, true);
    for (id, _, _, _) in &chats {
        call(
            &core,
            "config/reload",
            json!({"sessionId":id,"reloadRuntime":true}),
        )
        .await;
        wait_for_offer(&core, id, &api).await;
    }
    let ((first_id, _, _, _), (second_id, _, _, _)) = (&chats[0], &chats[1]);
    let (first, second) = tokio::join!(
        core.execute(
            crate::Origin::Ui,
            crate::Command::StartTurn {
                session_id: first_id.clone(),
                message: vec![json!({"type":"text","text":"After adding key, first."})],
                client_user_message_id: None,
            }
        ),
        core.execute(
            crate::Origin::Ui,
            crate::Command::StartTurn {
                session_id: second_id.clone(),
                message: vec![json!({"type":"text","text":"After adding key, second."})],
                client_user_message_id: None,
            }
        ),
    );
    first.unwrap();
    second.unwrap();
    for (index, (id, model, effort, policy)) in chats.iter().enumerate() {
        let request = api.wait_for(model).await;
        assert_eq!(request["model"], *model, "policy={policy}: {request}");
        assert_eq!(
            request["reasoning_effort"], *effort,
            "policy={policy}: {request}"
        );
        assert_eq!(saved(root.path(), id), before[index], "policy={policy}");
        assert_eq!(detached[index].0["model"], *model, "policy={policy}");
        assert_eq!(
            detached[index].0["reasoningEffort"], *effort,
            "policy={policy}"
        );
        assert_eq!(
            detached[index].1["runtime"]["config"]["activeModel"]["alias"], *model,
            "policy={policy}"
        );
        assert_eq!(
            detached[index].1["runtime"]["config"]["activeModel"]["thinking"], *effort,
            "policy={policy}"
        );
        wait_completed(&core, id).await;
    }
    assert!(api.authorized_saved.load(Ordering::Relaxed) >= 1);
    assert!(api.authorized_other.load(Ordering::Relaxed) >= 1);
    assert_eq!(saved_bytes(root.path()), before_bytes);
    eprintln!(
        "PINNED_315_UNLOCKPICK_TWO_CHATS no-key-add-key ask/accept/full distinct-models-and-effort"
    );
    core.shutdown().await;
    api.task.abort();
}
