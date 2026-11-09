//! Safari action receipts use the shared gateway and policy; fixtures never claim native browser proof.
use super::{gateway, output_tests, test_support};
use serde_json::json;

#[allow(clippy::unwrap_used, reason = "isolated test fixture initialization")]
async fn safari_core(label: &str) -> crate::Core {
    let core = output_tests::core(label).await;
    let shared = &core.shared;
    shared.chrome_control.owner.connected_profiles.observe(
        &json!({"profiles":[{"profileId":"safari:p","connected":true,"extensionVersion":"0.1.8"}]}),
    );
    std::fs::write(
        shared
            .chrome_control
            .dir()
            .unwrap()
            .join("safari-enabled.json"),
        b"true",
    )
    .unwrap();
    shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "safari:p", None);
    core
}

#[tokio::test]
async fn a_failed_find_keeps_the_same_safari_session_available() {
    let core = safari_core("safari-find-recovery").await;
    let shared = &core.shared;
    let calls = test_support::answering(shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::mcp(
                json!({"profiles":[{"profileId":"safari:p","connected":true,"features":["page.find","page.read"]}],"sessions":[{"sessionId":"t","profileId":"safari:p"}]}),
            );
        }
        let failed = args["args"]["query"] == "";
        output_tests::mcp(
            json!({"sessionId":"t","action":args["action"],"status":if failed {"failed"} else {"completed"},
            "error":if failed {json!({"code":"INVALID_ARGUMENT","message":"The search query is invalid. Supply a short query."})} else {json!(null)},
            "data":if failed {json!(null)} else {json!({"matches":[],"refs":[]})}}),
        )
    });
    for (query, expected) in [("", "failed"), ("absent", "completed")] {
        let reply = gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.find","args":{"query":query}}),
        )
        .await
        .unwrap();
        assert_eq!(
            reply["vwMcpResult"]["structuredContent"]["status"], expected,
            "{reply}"
        );
        assert_eq!(
            shared
                .chrome_control
                .work
                .lock()
                .unwrap()
                .state_of("t", "s"),
            Some("active")
        );
    }
    gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        3
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_session")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn safari_screenshot_keeps_shared_read_permission_and_mcp_image_receipt() {
    let core = safari_core("safari-capture-receipt").await;
    let shared = &core.shared;
    let request = json!({"sessionId":"t","action":"capture.screenshot","args":{"tabId":7}});
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    assert!(super::check::authorize(shared, "s", "chrome_execute", &request).is_ok());
    let mut helper = output_tests::mcp(
        json!({"sessionId":"t","action":"capture.screenshot","status":"completed","data":{"mimeType":"image/png","width":120,"height":40,"coordinateSpace":"viewport-css-pixels","cssScale":1,"captureRect":{"x":100,"y":80,"width":120,"height":40},"dataUrl":null}}),
    );
    helper["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"image","mimeType":"image/png","data":"YWJj"}));
    let result = super::output_cache::for_model(
        shared,
        "s",
        &request,
        super::owner::Answer::Helper(helper),
        "chrome_execute",
        false,
    );
    let body = &result["vwMcpResult"]["structuredContent"];
    assert_eq!(body["profileId"], "safari:p");
    assert_eq!(body["status"], "completed");
    assert_eq!(
        result["vwMcpResult"]["content"][1],
        json!({"type":"image","mimeType":"image/png","data":"YWJj"})
    );
    core.shutdown().await;
}

#[tokio::test]
async fn unsupported_safari_keys_keep_shared_policy_and_leave_the_session_active() {
    let core = safari_core("safari-key-limitation").await;
    let shared = &core.shared;
    for mode in ["ask", "accept-edits", "auto-approve"] {
        shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let args = json!({"sessionId":"t","action":"page.key","args":{"key":"Tab"}});
        assert!(super::check::authorize(shared, "s", "chrome_execute", &args).is_ok());
    }
    test_support::answering(shared, |_, _| {
        output_tests::mcp(
            json!({"sessionId":"t","action":"page.key","status":"failed","error":{"code":"UNSUPPORTED_ACTION","message":"Safari does not implement this action. Use another available browser if the task requires it."}}),
        )
    });
    let result = gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.key","args":{"key":"Enter"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["status"],
        "failed"
    );
    assert_eq!(
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .state_of("t", "s"),
        Some("active")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn safari_history_keeps_chrome_navigation_receipts_and_ask_authority() {
    let core = safari_core("safari-history-receipts").await;
    let shared = &core.shared;
    shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    let receipt = json!({"url":"https://destination.test/","title":"Destination","documentId":"history-document","origin":"https://destination.test","settled":true});
    let returned = receipt.clone();
    let calls = test_support::answering(shared, move |_, args| {
        output_tests::mcp(
            json!({"sessionId":"t","action":args["action"],"status":"completed","data":returned}),
        )
    });
    for action in ["page.back", "page.forward"] {
        let args = json!({"sessionId":"t","action":action,"args":{"tabId":7}});
        assert!(super::check::authorize(shared, "s", "chrome_execute", &args).is_ok());
        let reply = gateway::call(shared, "s", "chrome_execute", args)
            .await
            .unwrap();
        let body = &reply["vwMcpResult"]["structuredContent"];
        assert_eq!(body["status"], "completed");
        assert_eq!(body["profileId"], "safari:p");
        assert_eq!(body["untrustedPageContent"]["data"], receipt);
    }
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        2
    );
    core.shutdown().await;
}

#[test]
fn safari_capture_discovery_keeps_the_helpers_subset_when_chrome_is_also_connected() {
    let catalog = output_tests::catalog();
    let source = catalog["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "capture.screenshot")
        .unwrap();
    let full = source["inputSchema"].clone();
    let mut limited = full.clone();
    for key in ["fullPage", "clip", "filename"] {
        limited["properties"].as_object_mut().unwrap().remove(key);
    }
    limited["properties"]["saveTo"]["enum"] = json!(["inline"]);
    let reply = json!({"structuredContent":{"profiles":[
        {"profileId":"safari:p","connected":true,"features":["capture.screenshot"],"extensionCapabilities":["capture.screenshot"]},
        {"profileId":"chrome:p","connected":true,"features":["capture.screenshot"],"extensionCapabilities":["capture.screenshot"]}
    ],"capabilities":{"actions":[{"action":"capture.screenshot","inputSchema":limited,"profileSchemas":{"safari:p":limited,"chrome:p":full}}]}}});
    for (profile, all_shapes) in [("safari:p", false), ("chrome:p", true)] {
        let mut result = reply.clone();
        super::capabilities::filter(&mut result, Some(profile));
        let schema = &result["structuredContent"]["capabilities"]["actions"][0]["inputSchema"];
        for key in ["fullPage", "clip", "filename"] {
            assert_eq!(schema["properties"].get(key).is_some(), all_shapes);
        }
        if !all_shapes {
            assert_eq!(schema["properties"]["saveTo"]["enum"], json!(["inline"]));
        }
    }
}
