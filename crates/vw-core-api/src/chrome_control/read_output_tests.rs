//! Read-page budgets and explicit chat-folder exports through the real gateway with a deterministic helper.
//! No browser, model, shell or network; bytes include the app envelope and code-mode return indentation.
use super::{gateway, output_tests, test_support};
use serde_json::{Value, json};
use std::path::PathBuf;

async fn fixture(label: &str) -> (crate::Core, PathBuf) {
    let core = output_tests::core(label).await;
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
    (core, folder)
}

fn request(args: Value) -> Value {
    let mut request = json!({"sessionId":"t","action":"page.read"});
    request["args"] = args;
    request
}

fn captured() -> Value {
    json!({"operationId":"read","sessionId":"t","status":"completed","action":"page.read",
        "data":{"readId":"capture","tabId":7,"documentId":"document-a","text":"😀é\"\\\n".repeat(3000),
            "coverage":{"complete":false,"reason":"remaining_text"},
            "continuation":{"text":{"action":"page.read","args":{"tabId":7,"cursor":"capture:opaque/😀+=\\"},"reason":"remaining_text"}}}})
}

#[tokio::test]
async fn reads_send_a_small_byte_budget_and_leave_cursor_arguments_exact() {
    let (core, folder) = fixture("chrome-focused-default").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        output_tests::mcp(
            json!({"status":"completed","action":"page.read","data":{"readId":"r","text":"Small read"}}),
        )
    });
    for args in [
        json!({}),
        json!({"maxChars":4000}),
        json!({"find":"Email","frameId":2}),
    ] {
        gateway::call(&core.shared, "s", "chrome_execute", request(args))
            .await
            .unwrap();
    }
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        request(json!({"cursor":"h-opaque/😀+=\\","tabId":7})),
    )
    .await
    .unwrap();
    let calls = seen.lock().unwrap().clone();
    let reads: Vec<_> = calls
        .iter()
        .filter(|(tool, _)| tool == "chrome_execute")
        .collect();
    assert_eq!(reads.len(), 4);
    for (_, args) in &reads[..3] {
        assert_eq!(args["args"]["maxBytes"], 2000, "{args}");
        assert_eq!(args["args"]["limit"], 8, "{args}");
    }
    assert_eq!(
        reads[3].1["args"],
        json!({"cursor":"h-opaque/😀+=\\","tabId":7})
    );
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn saving_is_explicit_lossless_and_inside_the_chat_folder() {
    let (core, folder) = fixture("chrome-explicit-save").await;
    let raw = captured();
    let copy = raw.clone();
    let seen = test_support::answering(&core.shared, move |_, _| output_tests::mcp(copy.clone()));
    let inline = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
        .await
        .unwrap();
    assert!(inline["vwMcpResult"]["structuredContent"]["continuation"].is_object());
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        0,
        "ordinary reads create no files"
    );
    let mut args = request(json!({}));
    args["saveTo"] = json!("file");
    let saved = gateway::call(&core.shared, "s", "chrome_execute", args)
        .await
        .unwrap();
    let body = &saved["vwMcpResult"]["structuredContent"];
    let file = &body["savedToFile"];
    let path = folder.join(
        file["path"]
            .as_str()
            .expect("explicit save receipt with a path"),
    );
    assert_eq!(path.parent(), Some(folder.as_path()));
    let bytes = std::fs::read(&path).unwrap();
    let restored: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored["untrustedPageContent"]["data"], raw["data"]);
    assert!(restored["note"].is_string());
    assert!(restored["promptGuard"].is_object());
    assert_eq!(file["bytes"], bytes.len());
    assert_eq!(
        body["untrustedPageContent"]["continuation"],
        raw["data"]["continuation"]
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(_, args)| args.get("saveTo").is_none()),
        "saveTo is app-owned"
    );
    assert!(serde_json::to_string_pretty(body).unwrap().len() < 2000);
    core.shutdown().await;
}

#[tokio::test]
async fn invalid_exports_refuse_before_dispatch_or_writing() {
    let (core, folder) = fixture("chrome-save-validation").await;
    let seen = test_support::answering(&core.shared, |_, _| output_tests::mcp(captured()));
    for args in [
        json!({"sessionId":"t","action":"page.read","args":{},"saveTo":"../outside.json"}),
        json!({"sessionId":"t","action":"page.type","args":{"ref":"input","text":"x"},"saveTo":"file"}),
        json!({"sessionId":"t","steps":[{"action":"page.read","args":{}}],"saveTo":"file"}),
    ] {
        let result = gateway::call(&core.shared, "s", "chrome_execute", args).await;
        assert!(result.is_err(), "invalid saveTo must refuse");
    }
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    core.shutdown().await;
}

#[test]
fn chrome_catalog_advertises_the_app_save_option() {
    let tools = vw_agent_tools::chrome_tool_list();
    let execute = tools
        .iter()
        .find(|tool| tool["name"] == "chrome_execute")
        .unwrap();
    assert_eq!(
        execute["inputSchema"]["properties"]["saveTo"]["enum"],
        json!(["inline", "file"])
    );
    let description = execute["description"].as_str().unwrap();
    assert!(
        description.contains("saveTo")
            && description.contains("page.read")
            && description.contains("cursor")
    );
}

#[tokio::test]
async fn four_focused_read_pages_fit_one_code_mode_return() {
    let (core, folder) = fixture("chrome-four-read-pages").await;
    // Core startup retries the local-network policy after two seconds. Finish that independent
    // handshake before recording reads, even when a loaded test runner delays this test past it.
    test_support::answering(&core.shared, |tool, args| {
        assert_eq!(tool, "chrome_status");
        assert!(args.get("_netPolicyChange").is_some());
        output_tests::mcp(json!({"revision":1}))
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while core
            .shared
            .chrome_control
            .local_network_retry
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    core.shared.chrome_control.owner.stop().await;
    let seen = test_support::answering(&core.shared, |_, args| {
        let budget = args["args"]["maxBytes"].as_u64().unwrap_or(3000);
        let count = (budget.saturating_sub(600) / 140)
            .min(args["args"]["limit"].as_u64().unwrap_or(32)) as usize;
        let refs: Vec<_> = (0..count).map(|i| json!({"ref":format!("e{i:08x}"),"role":"button","tag":"button",
            "name":format!("Contact person {i} in the directory"),"disabled":false,"required":false,"frameId":0})).collect();
        output_tests::mcp(
            json!({"operationId":"read","sessionId":"t","status":"completed","action":"page.read",
            "data":{"readId":"capture","tabId":7,"documentId":"document-a","textPreview":"Directory contacts. ".repeat(16),"refs":refs,
                "coverage":{"complete":false},"continuation":{"refs":{"action":"page.read","args":{"tabId":7,"cursor":"h-next"},"reason":"remaining_refs"}}}}),
        )
    });
    let mut reads = Vec::new();
    for _ in 0..4 {
        let reply = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
            .await
            .unwrap();
        let body = reply["vwMcpResult"]["structuredContent"].clone();
        println!(
            "READ bytes: compact={}, pretty={}",
            body.to_string().len(),
            serde_json::to_string_pretty(&body).unwrap().len()
        );
        reads.push(body);
    }
    let returned = serde_json::to_string_pretty(&json!({"result":{"reads":reads}})).unwrap();
    println!(
        "FOUR READS: bytes={}, spills-at-16000={}",
        returned.len(),
        usize::from(returned.len() > 16_000)
    );
    assert!(
        returned.len() <= 16_000,
        "combined read pages spill: {}",
        returned.len()
    );
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    assert_eq!(seen.lock().unwrap().len(), 4);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(tool, _)| tool == "chrome_execute"),
        "unexpected helper calls: {:?}",
        seen.lock().unwrap()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_frozen_read_can_be_saved_without_another_browser_call() {
    let (core, folder) = fixture("chrome-save-frozen-read").await;
    let seen = test_support::answering(&core.shared, |_, _| output_tests::mcp(captured()));
    let first = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
        .await
        .unwrap();
    let mut save =
        request(first["vwMcpResult"]["structuredContent"]["continuation"]["args"].clone());
    save["saveTo"] = json!("file");
    let mut wrong = save.clone();
    wrong["sessionId"] = json!("t2");
    assert!(
        gateway::call(&core.shared, "s", "chrome_execute", wrong)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    let saved = gateway::call(&core.shared, "s", "chrome_execute", save.clone())
        .await
        .unwrap();
    let path = saved["vwMcpResult"]["structuredContent"]["savedToFile"]["path"]
        .as_str()
        .unwrap();
    let restored: Value =
        serde_json::from_slice(&std::fs::read(folder.join(path)).unwrap()).unwrap();
    assert_eq!(restored["untrustedPageContent"]["data"], captured()["data"]);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "saving a cached read never rereads the browser"
    );
    core.shared.chrome_control.end_chat("s", false);
    assert!(
        gateway::call(&core.shared, "s", "chrome_execute", save)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn failed_reads_and_denied_modes_never_create_exports() {
    let (core, folder) = fixture("chrome-save-refusal").await;
    let seen = test_support::answering(&core.shared, |_, _| {
        output_tests::mcp(json!({"status":"failed","error":{"code":"CONTROL_SUSPENDED"}}))
    });
    let mut args = request(json!({}));
    args["saveTo"] = json!("file");
    let failed = gateway::call(&core.shared, "s", "chrome_execute", args.clone())
        .await
        .unwrap();
    assert_eq!(
        failed["vwMcpResult"]["structuredContent"]["status"],
        "failed"
    );
    assert!(
        failed["vwMcpResult"]["structuredContent"]
            .get("savedToFile")
            .is_none()
    );
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"vw-plan"}),
    );
    assert!(
        gateway::call(&core.shared, "s", "chrome_execute", args)
            .await
            .is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn exporting_a_failed_buffer_keeps_the_refusal_and_creates_no_file() {
    for kind in ["status", "unknown_error", "mcp_error"] {
        let (core, folder) = fixture(&format!("chrome-save-failed-buffer-{kind}")).await;
        test_support::answering(&core.shared, move |_, _| {
            let mut failed = captured();
            if kind == "status" {
                failed["status"] = json!("failed");
            } else if kind == "unknown_error" {
                failed["error"] = json!({"code":"OTHER","message":"The read failed."});
            }
            let mut result = output_tests::mcp(failed);
            result["isError"] = json!(kind == "mcp_error");
            result
        });
        let first = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
            .await
            .unwrap();
        let mut args =
            request(first["vwMcpResult"]["structuredContent"]["continuation"]["args"].clone());
        args["saveTo"] = json!("file");
        let result = gateway::call(&core.shared, "s", "chrome_execute", args).await;
        assert!(
            result.is_err(),
            "a failed buffered read must not become a successful export receipt: {kind}"
        );
        let reason = result.unwrap_err();
        assert!(
            reason.contains("failed") && reason.contains("omit saveTo"),
            "{reason}"
        );
        assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn exporting_a_non_read_buffer_refuses_without_replaying_it() {
    let (core, folder) = fixture("chrome-save-non-read-buffer").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        output_tests::mcp(captured())
    });
    let mut original = request(json!({"text":"contact"}));
    original["action"] = json!("page.find");
    let first = gateway::call(&core.shared, "s", "chrome_execute", original)
        .await
        .unwrap();
    let mut args =
        request(first["vwMcpResult"]["structuredContent"]["continuation"]["args"].clone());
    args["saveTo"] = json!("file");
    let result = gateway::call(&core.shared, "s", "chrome_execute", args).await;
    assert!(result.is_err(), "only page.read buffers support export");
    assert!(result.unwrap_err().contains("omit saveTo"));
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn export_respects_the_current_file_write_mode_for_fresh_and_buffered_reads() {
    let (core, folder) = fixture("chrome-save-write-mode").await;
    let seen = test_support::answering(&core.shared, |_, _| output_tests::mcp(captured()));
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    let first = gateway::call(&core.shared, "s", "chrome_execute", request(json!({})))
        .await
        .unwrap();
    for read_args in [
        json!({}),
        first["vwMcpResult"]["structuredContent"]["continuation"]["args"].clone(),
    ] {
        let mut args = request(read_args);
        args["saveTo"] = json!("file");
        let result = gateway::call(&core.shared, "s", "chrome_execute", args).await;
        assert!(result.is_err(), "Ask mode must not create an export");
        assert!(result.unwrap_err().contains("omit saveTo"));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"accept-edits"}),
    );
    let mut args = request(json!({}));
    args["saveTo"] = json!("file");
    let result = gateway::call(&core.shared, "s", "chrome_execute", args)
        .await
        .unwrap();
    assert!(result["vwMcpResult"]["structuredContent"]["savedToFile"].is_object());
    assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn an_unavailable_chat_folder_gives_a_reason_and_an_inline_option() {
    let core = output_tests::core("chrome-save-no-folder").await;
    let seen =
        test_support::answering(&core.shared, |_, _| panic!("missing folder reached helper"));
    let mut args = request(json!({}));
    args["saveTo"] = json!("file");
    let reason = gateway::call(&core.shared, "s", "chrome_execute", args)
        .await
        .unwrap_err();
    assert!(
        reason.contains("no folder") && reason.contains("omit saveTo"),
        "{reason}"
    );
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn losing_the_folder_or_stopping_during_a_read_never_claims_a_save() {
    for stop in [false, true] {
        let (core, folder) = fixture(&format!("chrome-save-race-{stop}")).await;
        let weak = std::sync::Arc::downgrade(&core.shared);
        let target = folder.clone();
        test_support::answering(&core.shared, move |_, _| {
            if stop {
                crate::stops::stopped(&weak.upgrade().unwrap(), "s");
            } else {
                std::fs::remove_dir(&target).unwrap();
            }
            output_tests::mcp(captured())
        });
        let mut args = request(json!({}));
        args["saveTo"] = json!("file");
        let result = gateway::call(&core.shared, "s", "chrome_execute", args).await;
        if stop {
            assert!(result.is_err());
            assert_eq!(std::fs::read_dir(folder).unwrap().count(), 0);
        } else {
            let result = result.unwrap();
            let body = &result["vwMcpResult"]["structuredContent"];
            assert_eq!(body["error"]["code"], "SAVE_FAILED");
            assert!(
                body["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("omit saveTo")
            );
            assert!(body.get("savedToFile").is_none());
            assert!(!folder.exists());
        }
        core.shutdown().await;
    }
}
