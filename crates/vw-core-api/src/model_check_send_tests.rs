//! Send and Debug API startTurn distinguish an unusable key from an unfetched account list.
//! Uses an isolated key service and a controlled chat RPC peer; no turn may reach that peer.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const KEY_NOT_READY: &str = "The saved model for this chat cannot be checked until a Mistral key is ready. Unlock the vault or add a Mistral key, then try Send again.";
pub(super) const NOT_FETCHED: &str = "The saved model for this chat cannot be checked because Vibe Work could not reach Mistral. Check the network or proxy settings, then try Send again.";

fn attach_peer(core: &crate::Core, policy: &str) -> tokio::task::JoinHandle<()> {
    let row = json!({"alias":"mistral-large-4-0","name":"mistral-large-4-0",
        "source":"local","thinking":"off"});
    let runtime = json!({"activeApprovalPolicy":{"name":policy},
        "config":{"models":[row.clone()],"activeModel":row}});
    core.shared.access.observe_runtime("chat", &runtime);
    let (client, server) = tokio::io::duplex(16384);
    let (read, write) = tokio::io::split(client);
    let connection =
        vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
    core.shared
        .channels
        .bind_test_peer("chat", connection, (0, 1));
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "runtime/read" => json!({"runtime":runtime,"ready":true}),
                "config/reload" => {
                    assert_eq!(
                        request["params"],
                        json!({"sessionId":"chat","reloadRuntime":true})
                    );
                    // Pinned Vibe returns an applied runtime even when its catalog fetch fails.
                    // The list stays local-only, and writes/turns still hit the panic below.
                    json!({"runtime":runtime,"status":"applied","strippedHistoryImages":0})
                }
                "config/fields/read" => json!({"fields":[],"targets":[]}),
                "session/ready/wait" => json!({"ready":true}),
                method => {
                    panic!("Send must be refused before a write or turn reaches Vibe: {method}")
                }
            };
            if let Some(id) = request.get("id") {
                let reply = json!({"jsonrpc":"2.0","id":id,"result":result});
                write
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    })
}

async fn assert_send_refusal(usable_key: bool, debug_api: bool) {
    for policy in ["ask", "accept-edits", "auto-approve"] {
        refusal_in_policy(usable_key, debug_api, policy).await;
    }
}

async fn refusal_in_policy(usable_key: bool, debug_api: bool, policy: &str) {
    let base =
        std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let root = tempfile::tempdir_in(base).unwrap();
    let keys = crate::refresh_notice_tests::memory_keys(root.path()).await;
    keys.store_key(vw_vault::SecretString::new(
        "offlinecatalog-send-test-00000000000",
    ))
    .await
    .unwrap();
    if !usable_key {
        keys.lock();
    }
    assert_eq!(keys.speech_available(), usable_key);
    let sessions = root.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join("model-choices.json");
    let saved = json!({"remembered":{"alias":"mistral-medium-latest",
        "name":"mistral-medium-latest","effort":"high","provider":"mistral",
        "provider_key":"offlinecatalog-saved-route"}});
    std::fs::write(
        &file,
        json!({"version":1,"chats":{"chat":saved}}).to_string(),
    )
    .unwrap();
    let before = std::fs::read(&file).unwrap();
    let name = format!("offlinecatalog-send-{usable_key}-{debug_api}-{policy}");
    let core = crate::offline_core::start_with(&name, move |config, _| {
        config.keys = Some(keys);
        config.sessions_dir = Some(sessions);
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"chat","cwd":core.shared.default_cwd,
                "status":{"type":"idle"}},"entries":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let peer = attach_peer(&core, policy);
    let mut events = core.subscribe(Some("chat".into()));
    let error = send_error(&core, "chat", debug_api).await;
    assert_eq!(error.code, crate::ErrorCode::InvalidParams, "{error}");
    assert_eq!(
        error.message,
        if usable_key {
            NOT_FETCHED
        } else {
            KEY_NOT_READY
        }
    );
    assert_eq!(std::fs::read(file).unwrap(), before);
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
            )),
        "missing account models must not raise a model-choice notice"
    );
    assert!(
        !peer.is_finished(),
        "the controlled peer must not have panicked"
    );
    eprintln!(
        "OFFLINE_SEND_REASON_CASE=PASS usable_key={usable_key} debug_api={debug_api} policy={policy}"
    );
    core.shutdown().await;
    peer.abort();
}

pub(super) async fn send_error(core: &crate::Core, id: &str, debug_api: bool) -> crate::CoreError {
    let (origin, command) = if debug_api {
        (
            crate::Origin::DebugApi,
            serde_json::from_value(json!({"type":"startTurn","sessionId":id,
                "message":[{"type":"text","text":"Hello"}]}))
            .unwrap(),
        )
    } else {
        (
            crate::Origin::Ui,
            crate::Command::Call {
                method: "turn/start".into(),
                params: json!({"sessionId":id,"message":[{"type":"text","text":"Hello"}]}),
                confirmation_token: None,
            },
        )
    };
    core.execute(origin, command).await.unwrap_err()
}

#[tokio::test]
async fn send_with_locked_key_keeps_the_key_ready_sentence() {
    assert_send_refusal(false, false).await;
}

#[tokio::test]
async fn debug_api_start_turn_with_locked_key_keeps_the_key_ready_sentence() {
    assert_send_refusal(false, true).await;
}

#[tokio::test]
async fn send_with_usable_key_and_unfetched_catalog_uses_the_network_sentence() {
    assert_send_refusal(true, false).await;
}

#[tokio::test]
async fn debug_api_start_turn_with_usable_key_and_unfetched_catalog_uses_the_network_sentence() {
    assert_send_refusal(true, true).await;
}
