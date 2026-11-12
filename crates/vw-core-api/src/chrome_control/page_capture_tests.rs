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
