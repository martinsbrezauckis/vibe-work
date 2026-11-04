//! Observation catalog, authority and lossless output contracts; runs the app gateway with helper-shaped data.
use super::{gateway, output_tests, test_support};
use serde_json::{Value, json};

#[test]
fn observation_catalog_includes_find_and_optional_arguments() {
    let catalog = output_tests::catalog();
    let rows = catalog["actions"].as_array().unwrap();
    assert_eq!(rows.len(), 66);
    for (action, keys) in [
        (
            "page.read",
            vec!["includeText", "cursor", "scope", "includeBounds"],
        ),
        (
            "page.find",
            vec!["query", "cursor", "tabId", "frameId", "ref", "limit"],
        ),
        ("page.click", vec!["observation"]),
        ("page.type", vec!["observation"]),
        ("page.select", vec!["observation"]),
    ] {
        let row = rows.iter().find(|r| r["id"] == action).unwrap();
        for key in keys {
            assert!(
                row["inputSchema"]["properties"].get(key).is_some(),
                "{action}: {key}"
            );
        }
    }
}

#[tokio::test]
async fn find_has_the_read_permission_in_every_browser_mode() {
    let core = output_tests::core("chrome-find-modes").await;
    for mode in ["ask", "accept-edits", "auto-approve", "vw-plan", "explore"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let read = super::check::authorize(
            &core.shared,
            "s",
            "chrome_execute",
            &json!({"action":"page.read"}),
        );
        let find = super::check::authorize(
            &core.shared,
            "s",
            "chrome_execute",
            &json!({"action":"page.find"}),
        );
        assert_eq!(read.is_ok(), find.is_ok(), "{mode}: {find:?}");
        if let Ok(tier) = find {
            assert!(super::mapping::capabilities(tier).contains(&"page.read"));
            assert!(!super::mapping::capabilities(tier).contains(&"page.find"));
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn execute_unknown_action_lists_grouped_names_without_helper_dispatch() {
    let core = output_tests::core("chrome-unknown-execute").await;
    let seen =
        test_support::answering(&core.shared, |_, _| panic!("unknown action reached helper"));
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"unsupported.action","args":{}}),
    )
    .await
    .unwrap();
    let error = &answer["vwMcpResult"]["structuredContent"]["error"];
    assert_eq!(error["code"], "UNKNOWN_ACTION");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("page:") && message.contains("find") && message.contains("Use group.name")
    );
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

fn receipt() -> Value {
    // Shape from the helper's observation_contract_tests: large fields and a frozen helper cursor.
    json!({"operationId":"op","sessionId":"t","status":"completed","action":"page.read",
        "capability":"page.read","policyRevision":3,"leaseEpoch":9,"error":null,
        "data":{"readId":"r","revision":12,"tabId":1,"documentId":"doc",
            "refs":[{"ref":"huge","value":"😀é\\\"\n".repeat(6000),"name":"Large field"}],
            "coverage":{"complete":false,"reason":"receipt_budget"},
            "continuation":{"helper":{"action":"page.read","args":{"tabId":1,"cursor":"h:Opaque/😀+=\\token"},"reason":"receipt_budget"}}}})
}

#[tokio::test]
async fn outer_buffer_reassembles_original_before_helper_continuation() {
    let core = output_tests::core("chrome-buffer-roundtrip").await;
    let original = receipt();
    let copy = original.clone();
    let seen = test_support::answering(&core.shared, move |_, _| output_tests::mcp(copy.clone()));
    let mut answer = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    let mut restored = String::new();
    for _ in 0..300 {
        let body = &answer["vwMcpResult"]["structuredContent"];
        assert!(super::output::fits(body, 4000));
        let details = &body["untrustedPageContent"]["details"];
        if details.is_object() {
            assert_eq!(details["offset"], restored.encode_utf16().count());
            restored.push_str(details["text"].as_str().unwrap());
        }
        let Some(next) = body.get("continuation") else {
            break;
        };
        assert_eq!(next["action"], "page.read");
        let args = next["args"].clone();
        answer = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.read","args":args}),
        )
        .await
        .unwrap();
    }
    let restored: Value = serde_json::from_str(&restored).expect("app must buffer omitted content");
    super::observation_output_tests::assert_receipt(&restored, &original);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "outer reads must not consume the helper cursor"
    );
    core.shutdown().await;
}
