//! Combined status lookup through the app gateway, including legacy helpers and the actual model envelope.
use super::{gateway, output_tests, test_support};
use serde_json::json;

fn strip(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            if m.get("description")
                .is_some_and(serde_json::Value::is_string)
            {
                m.remove("description");
            }
            for v in m.values_mut() {
                strip(v);
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                strip(v);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn eight_legacy_action_schemas_fit_without_losing_constraints() {
    for modern in [false, true] {
        let core = output_tests::core("chrome-multiple-schemas").await;
        let seen = test_support::answering(&core.shared, move |tool, args| {
            assert_eq!(tool, "chrome_status");
            let mut result = output_tests::status(args);
            for profile in result["structuredContent"]["profiles"]
                .as_array_mut()
                .unwrap()
            {
                profile["extensionVersion"] = json!("0.1.4");
                profile["extensionCapabilities"] = if modern {
                    json!([
                        "page.read.includeText",
                        "page.read.cursor",
                        "page.read.scope.viewport",
                        "page.read.includeBounds",
                        "page.observation.changes",
                        "page.key.frameId",
                        "page.read.summary",
                        "page.find.scope.viewport"
                    ])
                } else {
                    json!([])
                };
            }
            result
        });
        let actions = [
            "page.read",
            "page.find",
            "page.click",
            "page.type",
            "page.select",
            "page.key",
            "page.scroll",
            "files.upload",
        ];
        let result = gateway::call(
            &core.shared,
            "s",
            "chrome_status",
            json!({"actions":actions}),
        )
        .await
        .unwrap();
        let body = &result["vwMcpResult"]["structuredContent"];
        let rows = body["untrustedPageContent"]["actions"]
            .as_array()
            .expect("combined schemas");
        assert_eq!(rows.len(), 8);
        for row in rows {
            assert!(row["inputSchema"].is_object());
            assert!(row.get("supported").is_some());
        }
        let read = rows.iter().find(|r| r["action"] == "page.read").unwrap();
        assert_eq!(
            read["inputSchema"]["properties"]
                .get("includeText")
                .is_some(),
            modern
        );
        assert_eq!(read["inputSchema"]["properties"]["limit"]["maximum"], 500);
        let key = rows.iter().find(|r| r["action"] == "page.key").unwrap();
        assert_eq!(
            key["inputSchema"]["properties"].get("frameId").is_some(),
            modern
        );
        let (compact, pretty) = output_tests::sizes(&result);
        println!("EIGHT SCHEMAS modern={modern} compact={compact} pretty={pretty}");
        assert!(compact < 9000 && pretty < if modern { 15000 } else { 13000 });
        assert!(body.get("outputLimit").is_none());
        for row in rows {
            let single = gateway::call(
                &core.shared,
                "s",
                "chrome_status",
                json!({"action":row["action"]}),
            )
            .await
            .unwrap();
            let mut expected = output_tests::schema(
                &single["vwMcpResult"]["structuredContent"]["untrustedPageContent"],
            );
            strip(&mut expected);
            assert_eq!(
                row["inputSchema"], expected,
                "all executable constraints survive"
            );
        }
        assert!(!seen.lock().unwrap().is_empty());
        core.shutdown().await;
    }
}

#[tokio::test]
async fn invalid_combined_lookups_never_reach_the_helper() {
    let core = output_tests::core("chrome-invalid-multiple-schemas").await;
    let seen =
        test_support::answering(&core.shared, |_, _| panic!("invalid lookup reached helper"));
    for args in [
        json!({"actions":[]}),
        json!({"actions":["page.read","page.read"]}),
        json!({"actions":["unknown"]}),
        json!({"actions":["page.read"],"action":"page.read"}),
        json!({"actions":["page.read"],"_extra":true}),
        json!({"actions":vec!["page.read";9]}),
    ] {
        assert!(
            gateway::call(&core.shared, "s", "chrome_status", args)
                .await
                .is_err()
        );
    }
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn oversized_schema_combination_is_explicitly_refused_without_partial_rows() {
    let core = output_tests::core("chrome-oversized-multiple-schemas").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = output_tests::status(args);
        for profile in result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap()
        {
            profile["extensionCapabilities"] = json!([
                "page.read.includeText",
                "page.read.cursor",
                "page.read.scope.viewport",
                "page.read.includeBounds",
                "page.observation.changes"
            ]);
        }
        result
    });
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_status",
        json!({"actions":["page.click","network.intercept","capture.screenshot","page.read",
            "page.wait","page.select","capture.zoom","page.type"]}),
    )
    .await;
    assert!(result.unwrap_err().contains("no schemas were omitted"));
    core.shutdown().await;
}
#[test]
fn multi_action_discovery_accepts_an_explicit_browser() {
    assert!(
        super::status_actions::requested(
            &serde_json::json!({"profileId":"edge","actions":["page.read"]})
        )
        .is_ok()
    );
}
