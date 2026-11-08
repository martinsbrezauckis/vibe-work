//! Gateway output budgets against shipped-helper-shaped fixtures; no browser or helper process.
//! Tests the actual MCP text and the sorted, indented JSON Vibe uses in code mode.
use super::{content, gateway, owner, test_support};
use serde_json::{Value, json};

pub(super) fn catalog() -> Value {
    serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap()
}

pub(super) fn mcp(body: Value) -> Value {
    let mut result = json!({"isError":false,"content":[{"type":"text","text":body.to_string()}]});
    result["structuredContent"] = body;
    result
}

pub(super) fn status(args: &Value) -> Value {
    let catalog = catalog();
    let features: Vec<_> = catalog["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].clone())
        .collect();
    let actions: Vec<_> = catalog["actions"].as_array().unwrap().iter()
        .filter(|r| args.get("action").is_none_or(|a| a == &r["id"]))
        .map(|r| if args.get("action").is_some() {
            json!({"action":r["id"],"capability":r["capability"],"description":r["description"],"inputSchema":r["inputSchema"]})
        } else { json!({"action":r["id"],"capability":r["capability"]}) }).collect();
    let profiles: Vec<_> = (0..2).map(|i| json!({
        "profileId":format!("chrome:profile-{i}"),"name":format!("Profile {i}"),"browser":"chrome",
        "connected":true,"browserVersion":"142.0.7444.60","extensionVersion":"0.1.6",
        "extensionCapabilities":features,"features":features,"route":"extension"
    })).collect();
    mcp(json!({"profiles":profiles,"sessions":[
        {"sessionId":"t","profileId":"chrome:profile-0","state":"active","purpose":"Read the form", "target":{"tabId":7,"url":"https://example.test/form","documentId":"document-a"}},
        {"sessionId":"t2","profileId":"chrome:profile-1","state":"active","purpose":"Check the form", "target":{"tabId":8,"url":"https://example.test/form","documentId":"document-b"}},
        {"sessionId":"foreign","profileId":"chrome:profile-0","state":"active"}
    ],"pending":[],"chromeConnected":true,"owner":{"key":"a".repeat(64),"fingerprint":"b".repeat(64)},
        "message":"Chrome is connected. Use only a connected profileId returned here.",
        "capabilities":{"route":"extension","actions":actions,"permissions":catalog["capabilities"],
        "availability":"Only actions advertised in a connected profile's features are supported by that browser session. The chat access mode and Chrome install grants apply. Use chrome_status with action for its argument schema.",
        "savedLogin":"auth.fill uses a named Vibe Work vault login and the vault’s own native consent. Chrome password-manager extraction is not provided.",
        "execution":"Actions return an observed receipt; permissions follow the chat access mode. Higher-tier actions return a refusal. A timeout cancels that call; an unknown outcome remains unresolved."}}))
}

pub(super) fn page() -> Value {
    let refs: Vec<_> = (0..40).map(|i| json!({"ref":format!("e{i:08x}"),"role":"textbox","tag":"input","name":format!("Form field {i}"),"frameId":2})).collect();
    mcp(
        json!({"operationId":"op-read","sessionId":"t","profileId":"chrome:profile-0",
        "status":"completed","action":"page.read","capability":"page.read","policyRevision":3,"leaseEpoch":9,"error":null,
        "data":{"tabId":7,"documentId":"document-a","origin":"https://example.test","url":"https://example.test/form","title":"Large form",
        "text":"Visible form text. ".repeat(440),"textOffset":0,"nextTextOffset":7920,
        "refs":refs,"offset":0,"totalRefs":240,"nextOffset":40,"scanTruncated":false,
        "frames":[{"frameId":2,"documentId":"child-document","origin":"https://child.test"}],
        "structure":[{"frameId":0,"headings":[{"level":1,"text":"Large form"}],"lists":[],"tables":[],"landmarks":[]}]}}),
    )
}

pub(super) async fn core(label: &str) -> crate::core::Core {
    let core = crate::offline_core::start_with(label, |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    for id in ["t", "t2"] {
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert(id.into(), "s".into());
    }
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            status(args)
        } else {
            page()
        }
    });
    core
}

pub(super) fn schema(row: &Value) -> Value {
    serde_json::from_str(
        row["inputSchemaJson"]
            .as_str()
            .expect("compact schema JSON"),
    )
    .unwrap()
}

pub(super) fn sizes(answer: &Value) -> (usize, usize) {
    let blocks = answer["vwMcpResult"]["content"].as_array().unwrap();
    let mut compact = 0;
    let mut pretty = 0;
    for block in blocks.iter().filter(|b| b["type"] == "text") {
        let text = block["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        compact += text.chars().count();
        pretty += serde_json::to_string_pretty(&body).unwrap().chars().count();
    }
    (compact, pretty)
}

fn fits(label: &str, answer: &Value, budget: usize) {
    let (compact, pretty) = sizes(answer);
    println!("SIZE {label}: compact={compact}, pretty={pretty}, budget={budget}");
    assert!(
        compact <= budget && pretty <= 16_000,
        "{label}: compact={compact} > {budget} or pretty={pretty} > 16000"
    );
}

#[tokio::test]
async fn chrome_status_overview_inline_budget() {
    assert_eq!(catalog()["capabilities"].as_array().unwrap().len(), 31);
    let core = core("chrome-output-status").await;
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    core.shutdown().await;
    fits("chrome_status({})", &answer, 6000);
}

#[tokio::test]
async fn overview_keeps_mode_exceptions_and_shared_profile_lists() {
    let core = core("chrome-output-mode-tables").await;
    for mode in ["ask", "accept-edits", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
            .await
            .unwrap();
        fits(mode, &answer, 6000);
        let status = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
        assert_eq!(status["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(status["featureLists"].as_array().unwrap().len(), 1);
        for p in status["profiles"].as_array().unwrap() {
            let mut restored = status["profileDefaults"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            restored.extend(p.as_object().unwrap().clone());
            assert_eq!(restored["features"], json!({"list":0}));
            assert_eq!(restored["extensionCapabilities"], json!({"list":0}));
        }
        assert_eq!(status["capabilities"]["permissions"]["total"], 31);
        let actions = &status["capabilities"]["actions"];
        assert_eq!(actions.as_object().unwrap().len(), 66);
        assert!(actions["page.read"].get("needs").is_none());
        assert_eq!(actions["page.type"].get("needs").is_some(), mode == "ask");
        assert_eq!(
            actions["raw.execute"].get("needs").is_some(),
            mode != "auto-approve"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn four_action_lookups_fit_a_code_mode_return() {
    let core = core("chrome-output-four-actions").await;
    let mut results = Vec::new();
    for action in ["page.read", "page.click", "page.type", "page.select"] {
        let answer = gateway::call(&core.shared, "s", "chrome_status", json!({"action":action}))
            .await
            .unwrap();
        results.push(answer["vwMcpResult"]["structuredContent"].clone());
    }
    let chars = serde_json::to_string_pretty(&json!({"results":results}))
        .unwrap()
        .chars()
        .count();
    println!("SIZE four action lookups in code mode: pretty={chars}");
    core.shutdown().await;
    assert!(chars <= 16000);
}

#[tokio::test]
async fn full_session_records_keep_their_shared_context() {
    let core = core("chrome-output-full-sessions").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = status(args);
        for row in result["structuredContent"]["sessions"]
            .as_array_mut()
            .unwrap()
        {
            row.as_object_mut().unwrap().extend(json!({"clientId":"client","clientName":"Vibe Work",
                "clientSessionId":"s","desiredState":"active","leaseEpoch":1,"policyRevision":1,
                "scope":"ownWindow","capture":null,"lastConfirmed":null,"groupTabs":[],"unresolved":[],"suspensionReason":null}).as_object().unwrap().clone());
        }
        result
    });
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    let expected_addons = super::installation::addons(&core.shared);
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    core.shutdown().await;
    fits("status with full session records", &answer, 6000);
    let status = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
    assert_eq!(status["sessionDefaults"]["clientSessionId"], "s");
    assert_eq!(status["sessions"].as_array().unwrap().len(), 2);
    let capabilities = &status["capabilities"];
    assert_catalog_features(status);
    for action in ["history.read", "bookmarks.read"] {
        assert_eq!(capabilities["actions"][action]["approvalRequired"], true);
        assert!(capabilities["actions"][action].get("needs").is_none());
    }
    for action in catalog()["actions"].as_array().unwrap() {
        let id = action["id"].as_str().unwrap();
        let name = &capabilities["actions"][id]["capability"];
        let name = if let Some(index) = name.as_u64() {
            &capabilities["capabilityNames"][usize::try_from(index).unwrap()]
        } else if name.is_null() {
            assert_eq!(capabilities["defaultCapability"], "action ID");
            &action["id"]
        } else {
            name
        };
        assert_eq!(name, &action["capability"], "{id}");
    }
    let addons = &answer["vwMcpResult"]["structuredContent"]["addons"];
    assert_eq!(
        addons.as_array().unwrap().len(),
        expected_addons.as_array().unwrap().len()
    );
    for (actual, expected) in addons
        .as_array()
        .unwrap()
        .iter()
        .zip(expected_addons.as_array().unwrap())
    {
        let mut expected = expected.clone();
        // Only the duplicate state label is omitted; diagnosis, instruction, identity and version stay exact.
        assert_eq!(expected["label"], expected["state"]);
        expected.as_object_mut().unwrap().remove("label");
        assert_eq!(actual, &expected);
    }
    if cfg!(feature = "your-edge") {
        assert_eq!(addons[1]["stateCode"], "notSetUp");
        assert!(capabilities["capabilityNames"].is_array());
    }
    if cfg!(all(feature = "your-safari", feature = "your-edge")) {
        // Crowded optional builds retain all per-profile facts through the same explicit-defaults convention.
        for (index, profile) in status["profiles"].as_array().unwrap().iter().enumerate() {
            let mut restored = status["profileDefaults"].as_object().unwrap().clone();
            restored.extend(profile.as_object().unwrap().clone());
            assert_eq!(restored["profileId"], format!("chrome:profile-{index}"));
            assert_eq!(restored["browser"], "chrome");
            assert_eq!(restored["connected"], true);
            assert_eq!(restored["browserVersion"], "142.0.7444.60");
            assert_eq!(restored["extensionVersion"], "0.1.6");
            assert_eq!(restored["route"], "extension");
            assert_eq!(restored["features"], json!({"list":0}));
            assert_eq!(restored["extensionCapabilities"], json!({"list":0}));
        }
    }
}

#[tokio::test]
async fn page_requests_clamp_and_say_so_once() {
    let core = core("chrome-output-clamp").await;
    let seen = test_support::answering(&core.shared, |_, _| {
        mcp(json!({"status":"completed","data":{"text":"Small","refs":[]}}))
    });
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{"maxChars":100_000,"maxBytes":100_000}}),
    )
    .await
    .unwrap();
    let calls = seen.lock().unwrap().clone();
    let sent = calls
        .iter()
        .find(|(tool, _)| tool == "chrome_execute")
        .unwrap();
    assert_eq!(sent.1["args"]["maxChars"], 4000);
    assert_eq!(sent.1["args"]["maxBytes"], 4000);
    let text = answer["vwMcpResult"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text.matches("capped at 4000").count(), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_status_every_action_inline_budget() {
    let core = core("chrome-output-actions").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = status(args);
        for profile in result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap()
        {
            profile["extensionCapabilities"]
                .as_array_mut()
                .unwrap()
                .extend([
                    json!("page.read.includeText"),
                    json!("page.read.cursor"),
                    json!("page.read.scope.viewport"),
                    json!("page.read.includeBounds"),
                    json!("page.observation.changes"),
                    json!("page.key.frameId"),
                    json!("page.read.summary"),
                    json!("page.find.scope.viewport"),
                ]);
        }
        result
    });
    let mut failures = Vec::new();
    for action in catalog()["actions"].as_array().unwrap() {
        let answer = gateway::call(
            &core.shared,
            "s",
            "chrome_status",
            json!({"action":action["id"]}),
        )
        .await
        .unwrap();
        let (compact, pretty) = sizes(&answer);
        let overrides: Value = serde_json::from_str(include_str!("action_catalog.json")).unwrap();
        let original = overrides["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == action["id"])
            .unwrap_or(action);
        let row = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
        assert_eq!(schema(row), original["inputSchema"], "{}", action["id"]);
        assert_eq!(
            row["description"], original["description"],
            "{}",
            action["id"]
        );
        assert_eq!(row["allowedInThisMode"], true);
        assert_eq!(row["supported"], true);
        println!("SIZE {}: compact={compact}, pretty={pretty}", action["id"]);
        // Encoding schema JSON adds escaped quotes on the wire, but removes its indent-two expansion.
        // Keep the original content budget too; the program test bounds the actual encoded aggregate.
        let mut decoded = answer["vwMcpResult"]["structuredContent"].clone();
        let decoded_row = &mut decoded["untrustedPageContent"];
        let schema = schema(decoded_row);
        decoded_row
            .as_object_mut()
            .unwrap()
            .remove("inputSchemaJson");
        decoded_row["inputSchema"] = schema;
        if decoded.to_string().chars().count() > 1500 || pretty > 2000 {
            failures.push((action["id"].clone(), compact, pretty));
        }
    }
    core.shutdown().await;
    assert!(failures.is_empty(), "action budgets: {failures:?}");
}

#[tokio::test]
async fn chrome_page_read_inline_budget() {
    let core = core("chrome-output-page").await;
    let mut failures = Vec::new();
    for args in [
        json!({}),
        json!({"limit":200,"maxChars":8000}),
        json!({"maxChars":60000,"maxBytes":64000}),
    ] {
        let answer = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.read","args":args}),
        )
        .await
        .unwrap();
        let (compact, pretty) = sizes(&answer);
        println!("SIZE page.read({args}): compact={compact}, pretty={pretty}");
        if compact > 4000 || pretty > 6000 {
            failures.push((args, compact, pretty));
        }
    }
    core.shutdown().await;
    assert!(failures.is_empty(), "page budgets: {failures:?}");
}

#[test]
fn small_receipt_control() {
    let raw =
        mcp(json!({"operationId":"op","status":"completed","data":{"text":"Small","refs":[]}}));
    let mut expected = raw.clone();
    content::mark(&mut expected);
    assert_eq!(
        content::for_model(owner::Answer::Helper(raw)).to_string(),
        expected.to_string()
    );
}

#[cfg(all(feature = "your-firefox", feature = "your-safari"))]
#[tokio::test]
async fn compact_catalog_reference_keeps_partial_profile_support_explicit() {
    let core = core("chrome-output-partial-features").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = status(args);
        for key in ["features", "extensionCapabilities"] {
            result["structuredContent"]["profiles"][1][key]
                .as_array_mut()
                .unwrap()
                .retain(|id| id != "raw.execute");
        }
        result
    });
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let status = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
    let lists = status["featureLists"].as_array().unwrap();
    assert_eq!(lists[0], json!({"allActions":true}));
    let partial = lists[1].as_array().unwrap();
    assert_eq!(partial.len(), 65);
    assert!(!partial.contains(&json!("raw.execute")));
    for action in catalog()["actions"].as_array().unwrap() {
        assert_eq!(
            partial.contains(&action["id"]),
            action["id"] != "raw.execute"
        );
    }
    core.shutdown().await;
}

fn assert_catalog_features(status: &Value) {
    let capabilities = &status["capabilities"];
    let features = &status["featureLists"][0];
    let mut actual: Vec<Value> = if features["allActions"] == true {
        assert!(
            capabilities["availability"]
                .as_str()
                .unwrap()
                .contains("allActions")
        );
        capabilities["actions"]
            .as_object()
            .unwrap()
            .keys()
            .map(|id| json!(id))
            .collect()
    } else {
        features.as_array().unwrap().clone()
    };
    let mut expected: Vec<Value> = catalog()["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].clone())
        .collect();
    actual.sort_by_key(Value::to_string);
    expected.sort_by_key(Value::to_string);
    assert_eq!(
        actual, expected,
        "every supported action survives compact status"
    );
}
