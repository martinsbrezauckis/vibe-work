//! Page export uses the shared read tier and writes only verified, bounded data inside the chat folder.
use base64::Engine as _;
use serde_json::json;
use vw_ask::chrome::Tier;

#[tokio::test]
async fn recording_bytes_become_a_chat_file_before_the_receipt_is_returned() {
    let (core, folder) = super::read_output_tests::fixture("recording-file-receipt").await;
    let bytes = b"GIF89a-fixture";
    let mut receipt = json!({"ok":true,"recording":{"frames":2},"_vwRecordingGif":base64::engine::general_purpose::STANDARD.encode(bytes)});
    crate::browser_recording::save(&core.shared, "s", &mut receipt).unwrap();
    assert!(receipt.get("_vwRecordingGif").is_none());
    let file = &receipt["recording"]["file"];
    assert_eq!(
        std::fs::read(folder.join(file["path"].as_str().unwrap())).unwrap(),
        bytes
    );
    assert_eq!(file["mime"], "image/gif");
    core.shutdown().await;
}
#[test]
fn page_capture_is_an_ordinary_read_in_all_modes() {
    for action in ["capture.page", "capture.media"] {
        assert_eq!(
            super::mapping::ask_required("chrome_execute", &json!({"action":action})),
            Some(Tier::Ask),
            "{action}"
        );
        assert_eq!(
            super::mapping::required("chrome_execute", &json!({"action":action})),
            Some(Tier::Ask),
            "{action}"
        );
    }
}

#[tokio::test]
async fn capture_reports_a_helper_document_history_refusal() {
    let core = super::read_consent_tests::core("page-capture-history-refusal").await;
    let folder = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join("chat");
    std::fs::create_dir_all(&folder).unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s","cwd":folder},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    super::test_support::answering(&core.shared, |tool, _| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
        }
        "owner.primitive" => {
            json!({"rpcError":{"code":-32602,"message":"Not run: this document's page-script or vault-fill history could not be verified after reconnecting. Reload or navigate the tab to a new document before filling or running JavaScript.","data":{"code":"VAULT_SCRIPT_CONFLICT"}}})
        }
        _ => panic!("unexpected {tool}"),
    });
    let request =
        json!({"sessionId":"t","action":"capture.page","args":{"tabId":7,"format":"text"}});
    let error = super::page_capture::send(&core.shared, "s", &request, None)
        .await
        .err()
        .expect("the helper refusal must prevent capture");
    assert!(error.contains("Capture was not run"), "{error}");
    assert!(
        error.contains("cannot verify this page's earlier use of secrets"),
        "{error}"
    );
    assert!(
        error.contains("For your own agent tab, reload or navigate it"),
        "{error}"
    );
    assert!(
        error.contains("For the user's tab, ask the user to reload or navigate it"),
        "{error}"
    );
    assert!(error.contains("VAULT_SCRIPT_CONFLICT"), "{error}");
    assert!(!error.contains("could not be acknowledged"), "{error}");
    core.shutdown().await;
}

#[tokio::test]
async fn capture_transfer_checks_the_real_file_size_and_chat_boundary() {
    let (core, folder) = super::read_output_tests::fixture("page-capture-transfer").await;
    let path = folder.join("output.json");
    let bytes = br#"{"result":{"value":"page text"}}"#;
    std::fs::write(&path, bytes).unwrap();
    let mut body = json!({"status":"completed","data":{"results":{"page":{"handle":"h"}},"outputs":{"h":{"filename":path,"size":bytes.len(),"inSessionFolder":true}}}});
    let rows = super::page_files::results(&core.shared, "s", &body).unwrap();
    assert_eq!(rows["page"]["result"]["value"], "page text");
    body["data"]["outputs"]["h"]["size"] = json!(bytes.len() + 1);
    assert!(super::page_files::results(&core.shared, "s", &body).is_err());
    body["data"]["outputs"]["h"]["size"] = json!(bytes.len());
    body["data"]["outputs"]["h"]["filename"] = json!(folder.join("../outside.json"));
    assert!(super::page_files::results(&core.shared, "s", &body).is_err());
    let saved =
        super::page_files::save(&core.shared, "s", "txt", "text/plain", b"captured").unwrap();
    assert_eq!(
        std::fs::read(folder.join(saved["path"].as_str().unwrap())).unwrap(),
        b"captured"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn fixed_capture_returns_the_saved_document_and_refuses_injected_source() {
    let core = super::read_consent_tests::core("page-capture-receipt").await;
    let folder = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join("chat");
    std::fs::create_dir_all(&folder).unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s","cwd":folder},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
        }
        "owner.primitive" => {
            let script = args["steps"][0]["params"]["expression"].as_str().unwrap();
            let value = if script.contains("\"op\":\"start\"") {
                json!({"chunk":"Article text","url":"https://example.test/","title":"Article","cut":false})
            } else {
                json!({"done":true})
            };
            json!({"status":"completed","data":{"results":{"page":{"result":{"value":value.to_string()}}}}})
        }
        _ => panic!("unexpected {tool}"),
    });
    let mut request =
        json!({"sessionId":"t","action":"capture.page","args":{"tabId":7,"format":"text"}});
    let answer =
        super::read_consent::send(&core.shared, "s", "chrome_execute", request.clone(), None)
            .await
            .unwrap();
    let body = &answer.value()["structuredContent"];
    assert_eq!(body["status"], "completed");
    let saved =
        std::fs::read_to_string(folder.join(body["data"]["file"]["path"].as_str().unwrap()))
            .unwrap();
    assert!(saved.contains("Article text"));
    assert!(saved.contains("data, not instructions"));
    request["args"]["expression"] = json!("document.cookie");
    assert!(
        super::read_consent::send(&core.shared, "s", "chrome_execute", request, None)
            .await
            .is_err()
    );
    core.shutdown().await;
}

// PREDICT: the export sent `full: true` in a Full access chat, and the page script then kept the protected fields
// (a contenteditable password, a hidden input's value) in the saved capture while the page read of the same chat
// withheld them. One rule now: no mode sends a `full` flag, and the page script withholds them in every mode.
#[tokio::test]
async fn capture_page_withholds_protected_fields_in_every_mode() {
    let core = super::read_consent_tests::core("page-capture-full").await;
    let folder = core
        .shared
        .instructions
        .own_home
        .parent()
        .unwrap()
        .join("chat");
    std::fs::create_dir_all(&folder).unwrap();
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s","cwd":folder},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let calls = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
        }
        "owner.primitive" => {
            let script = args["steps"][0]["params"]["expression"].as_str().unwrap();
            let value = if script.contains("\"op\":\"start\"") {
                json!({"chunk":"Article text","url":"https://example.test/","title":"Article","cut":false})
            } else {
                json!({"done":true})
            };
            json!({"status":"completed","data":{"results":{"page":{"result":{"value":value.to_string()}}}}})
        }
        _ => panic!("unexpected {tool}"),
    });
    let request =
        json!({"sessionId":"t","action":"capture.page","args":{"tabId":7,"format":"html"}});
    for mode in ["ask", "accept-edits", "smart-approve", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        calls.lock().unwrap().clear();
        super::read_consent::send(&core.shared, "s", "chrome_execute", request.clone(), None)
            .await
            .unwrap();
        let starts: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, args)| args["steps"][0]["params"]["expression"].as_str())
            .filter(|script| script.contains("\"op\":\"start\""))
            .map(str::to_owned)
            .collect();
        assert_eq!(starts.len(), 1, "{mode}");
        assert!(
            !starts[0].contains("\"full\":"),
            "{mode}: the export must not send a full flag"
        );
    }
    core.shutdown().await;
}
