//! Replacement confirmation follows the active key's latest test, using synthetic keys and loopback HTTP.
//! Exercises the real dispatcher, vault and confirmer; no account or installed desktop app is needed.

mod common;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::core::{ScriptRig, no_vibe_core_with as script_core_with};
use common::vault::key_service_config;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vw_core_api::{Command, CommandResult, ErrorCode, Origin};
use vw_gate::{Confirmation, Requester};
use vw_keys::KeyService;
use vw_vault::SecretString;

const ORIGINAL: &str = "synthetic-original-key";
const REPLACEMENT: &str = "synthetic-replacement-key";

fn store(value: &str) -> Command {
    serde_json::from_value(json!({
        "type": "keys", "request": {"op": "setupStoreKey", "apiKey": value}
    }))
    .unwrap()
}

fn test_key() -> Command {
    serde_json::from_value(json!({"type": "keys", "request": {"op": "testKey"}})).unwrap()
}

struct Rig {
    core: ScriptRig,
    keys: Arc<KeyService>,
    status: Arc<AtomicU16>,
    requests: Arc<Mutex<Vec<String>>>,
    arrived: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    server: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn new(name: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let status = Arc::new(AtomicU16::new(200));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let arrived = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (reply, seen) = (status.clone(), requests.clone());
        let (notify_arrival, held) = (arrived.clone(), release.clone());
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (reply, seen) = (reply.load(Ordering::SeqCst), seen.clone());
                let (notify_arrival, held) = (notify_arrival.clone(), held.clone());
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut buf = [0; 1024];
                    while !bytes.ends_with(b"\r\n\r\n") {
                        let n = stream.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buf[..n]);
                    }
                    seen.lock().unwrap().push(String::from_utf8(bytes).unwrap());
                    notify_arrival.notify_one();
                    // Zero disconnects; one holds the request beyond the production test timeout.
                    if reply == 0 {
                        return;
                    }
                    if reply == 1 {
                        tokio::time::sleep(Duration::from_secs(21)).await;
                        return;
                    }
                    let reply = if reply == 1200 {
                        held.notified().await;
                        200
                    } else {
                        reply
                    };
                    let body = r#"{"data":[]}"#;
                    let response = format!(
                        "HTTP/1.1 {reply} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        let dir = common::processes::scratch(name);
        let keys = KeyService::start(key_service_config(&dir, &base, Arc::default()))
            .await
            .unwrap();
        keys.create_vault(SecretString::new("synthetic test passphrase"))
            .await
            .unwrap();
        let core = script_core_with(name, &[], |config| config.keys = Some(keys.clone())).await;
        core.core
            .execute(Origin::Ui, store(ORIGINAL))
            .await
            .unwrap();
        assert!(core.confirmer.shown().is_empty());
        Self {
            core,
            keys,
            status,
            requests,
            arrived,
            release,
            server,
        }
    }

    async fn check(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
        let result = self.core.core.execute(Origin::Ui, test_key()).await;
        if status == 200 {
            assert!(matches!(result.unwrap(), CommandResult::KeyTest { .. }));
        } else {
            let error = result.unwrap_err();
            let reason = match status {
                401 => "unauthorized",
                403 => "forbidden",
                429 => "rateLimited",
                0 | 1 => "network",
                _ => "mistralServerError",
            };
            assert_eq!(error.server_code.as_deref(), Some(reason));
            if status == 1 {
                assert!(error.message.contains("timed out"));
            }
        }
    }

    async fn decline(&self) {
        let before = self.core.confirmer.shown().len();
        self.core.confirmer.then(Confirmation::Declined);
        let error = self
            .core
            .core
            .execute(Origin::Ui, store(REPLACEMENT))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfirmationDeclined);
        assert_eq!(error.message, "The Mistral key was kept.");
        let shown = self.core.confirmer.shown();
        assert_eq!(shown.len(), before + 1);
        assert_eq!(shown.last().unwrap().requester, Requester::Window);
        assert_eq!(shown.last().unwrap().method, "vibe-work/replaceModelKey");
        self.check(200).await;
        assert!(
            self.requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains(ORIGINAL)
        );
    }

    async fn shutdown(self) {
        self.core.core.shutdown().await;
        self.keys.shutdown().await;
        self.server.abort();
    }
}

#[tokio::test]
async fn rejected_key_is_replaced_without_calling_the_confirmer() {
    for status in [401, 403] {
        let rig = Rig::new(&format!("key-rejected-{status}")).await;
        rig.check(status).await;
        let before = rig.requests.lock().unwrap().len();
        // An unexpected confirmation would decline and fail the store.
        rig.core.confirmer.then(Confirmation::Declined);
        let result = rig.core.core.execute(Origin::Ui, store(REPLACEMENT)).await;
        assert!(
            result.is_ok(),
            "a rejected key must be replaced without confirmation: {result:?}"
        );
        assert!(rig.core.confirmer.shown().is_empty());
        assert_eq!(
            rig.requests.lock().unwrap().len(),
            before,
            "storage makes no network call"
        );
        rig.check(200).await;
        assert!(
            rig.requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .contains(REPLACEMENT)
        );
        rig.shutdown().await;
    }
}

#[tokio::test]
async fn working_and_untested_keys_still_confirm_and_decline_keeps_the_key() {
    let rig = Rig::new("key-working").await;
    rig.decline().await; // Never tested.
    rig.decline().await; // Latest test passed.
    rig.core.confirmer.then(Confirmation::Approved);
    rig.core
        .core
        .execute(Origin::Ui, store(REPLACEMENT))
        .await
        .unwrap();
    assert_eq!(rig.core.confirmer.shown().len(), 3);
    rig.check(200).await;
    assert!(
        rig.requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains(REPLACEMENT)
    );
    rig.shutdown().await;
}

#[tokio::test]
async fn network_timeout_and_rate_limit_failures_still_confirm() {
    for status in [0, 1, 429, 503] {
        let rig = Rig::new(&format!("key-transient-{status}")).await;
        rig.check(401).await;
        rig.check(status).await;
        rig.decline().await;
        rig.shutdown().await;
    }
}

#[tokio::test]
async fn a_later_success_supersedes_an_authentication_rejection() {
    let rig = Rig::new("key-test-recovery").await;
    rig.check(401).await;
    rig.check(200).await;
    rig.decline().await;
    rig.shutdown().await;
}

#[tokio::test]
async fn a_new_untested_key_does_not_inherit_the_previous_keys_rejection() {
    let rig = Rig::new("key-rejection-reset").await;
    rig.check(401).await;
    rig.keys
        .store_key(SecretString::new(ORIGINAL))
        .await
        .unwrap();
    rig.decline().await;
    rig.shutdown().await;
}

#[tokio::test]
async fn debug_api_cannot_replace_even_a_rejected_key() {
    let rig = Rig::new("key-rejected-origin").await;
    rig.check(401).await;
    let error = rig
        .core
        .core
        .execute(Origin::DebugApi, store(REPLACEMENT))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Denied);
    assert_eq!(error.server_code.as_deref(), Some("vaultUserOnly"));
    // Requesters are assigned by the transport; command input cannot impersonate the window.
    assert!(serde_json::from_value::<Origin>(json!("agent")).is_err());
    let mut spoofed = serde_json::to_value(store(REPLACEMENT)).unwrap();
    spoofed["requester"] = json!("window");
    assert!(serde_json::from_value::<Command>(spoofed).is_err());
    for method in ["keys/setupStoreKey", "vibe-work/replaceModelKey"] {
        let raw: Command = serde_json::from_value(json!({
            "type": "call", "method": method, "params": {}
        }))
        .unwrap();
        let refused = rig
            .core
            .core
            .execute(Origin::DebugApi, raw)
            .await
            .unwrap_err();
        assert_eq!(refused.code, ErrorCode::Denied);
    }
    assert!(rig.core.confirmer.shown().is_empty());
    rig.check(200).await;
    assert!(
        rig.requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains(ORIGINAL)
    );
    rig.shutdown().await;
}

#[tokio::test]
async fn an_old_in_flight_test_cannot_overwrite_the_new_keys_rejection() {
    let rig = Rig::new("key-in-flight-rotation").await;
    rig.status.store(1200, Ordering::SeqCst);
    let core = rig.core.core.clone();
    let old_test = tokio::spawn(async move { core.execute(Origin::Ui, test_key()).await });
    tokio::time::timeout(Duration::from_secs(5), rig.arrived.notified())
        .await
        .unwrap();
    rig.keys
        .store_key(SecretString::new(REPLACEMENT))
        .await
        .unwrap();
    rig.check(401).await;
    rig.release.notify_one();
    old_test.await.unwrap().unwrap();
    rig.core.confirmer.then(Confirmation::Declined);
    rig.core
        .core
        .execute(Origin::Ui, store(ORIGINAL))
        .await
        .unwrap();
    assert!(rig.core.confirmer.shown().is_empty());
    rig.shutdown().await;
}

#[tokio::test]
async fn agent_requester_cannot_reach_key_storage_through_the_method_gate() {
    let rig = common::gate::rig();
    for method in ["keys/setupStoreKey", "vibe-work/replaceModelKey"] {
        let result = rig
            .gate
            .request_confirmation_as(method, &json!({}), Requester::Agent)
            .await;
        assert!(matches!(result, Err(vw_gate::GateError::Denied { .. })));
        assert!(matches!(
            rig.gate.call(method, json!({}), None).await,
            Err(vw_gate::GateError::Denied { .. })
        ));
    }
    assert!(rig.transport.frames().is_empty());
    assert!(rig.confirmer.shown().is_empty());
}
