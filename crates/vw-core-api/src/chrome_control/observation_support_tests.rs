//! Accepted observation arguments and old/new helper negotiation; uses the gateway's mocked owned pipe.
use super::{gateway, output_tests, test_support};
use serde_json::{Value, json};

const NEW_KEYS: &[&str] = &[
    "includeText",
    "cursor",
    "scope",
    "includeBounds",
    "observation",
];

fn legacy_status(args: &Value) -> Value {
    let mut result = output_tests::status(args);
    let body = &mut result["structuredContent"];
    for profile in body["profiles"].as_array_mut().unwrap() {
        for key in ["features", "extensionCapabilities"] {
            profile[key]
                .as_array_mut()
                .unwrap()
                .retain(|action| action != "page.find");
        }
    }
    let rows = body["capabilities"]["actions"].as_array_mut().unwrap();
    rows.retain(|row| row["action"] != "page.find");
    for row in rows {
        if let Some(properties) = row["inputSchema"]["properties"].as_object_mut() {
            properties.retain(|key, _| !NEW_KEYS.contains(&key.as_str()));
        }
        row["description"] = json!("Legacy helper action description");
    }
    result
}

#[tokio::test]
async fn legacy_helper_does_not_advertise_or_receive_new_arguments() {
    let core = output_tests::core("chrome-legacy-observation-schema").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            legacy_status(args)
        } else {
            output_tests::mcp(json!({"status":"completed"}))
        }
    });
    for action in ["page.read", "page.click", "page.type", "page.select"] {
        let reply = gateway::call(&core.shared, "s", "chrome_status", json!({"action":action}))
            .await
            .unwrap();
        let row = &reply["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
        for key in NEW_KEYS {
            assert!(output_tests::schema(row)["properties"].get(*key).is_none());
        }
        assert_eq!(row["description"], "Legacy helper action description");
        let args = if action == "page.read" {
            json!({"includeText":false})
        } else {
            json!({"ref":"r","observation":{"mode":"changes"}})
        };
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":action,"args":args}),
        )
        .await;
        assert!(
            reply.is_err(),
            "old helper cannot accept {action}: {reply:?}"
        );
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "chrome_execute")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn accepted_arguments_reach_capable_extension_verbatim() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/observation_args.json")).unwrap();
    let catalog = output_tests::catalog();
    let core = output_tests::core("chrome-observation-args").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            let mut reply = output_tests::status(args);
            for profile in reply["structuredContent"]["profiles"]
                .as_array_mut()
                .unwrap()
            {
                profile["extensionCapabilities"] = json!([
                    "page.read",
                    "page.read.includeText",
                    "page.read.cursor",
                    "page.read.scope.viewport",
                    "page.read.includeBounds",
                    "page.observation.changes"
                ]);
            }
            reply
        } else {
            output_tests::mcp(json!({"status":"completed"}))
        }
    });
    for call in fixture["calls"].as_array().unwrap() {
        let schema = &catalog["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == call["action"])
            .unwrap()["inputSchema"];
        for key in call["args"].as_object().unwrap().keys() {
            assert!(schema["properties"].get(key).is_some(), "{call}: {key}");
        }
        let mut request = call.clone();
        request["sessionId"] = json!("t");
        gateway::call(&core.shared, "s", "chrome_execute", request)
            .await
            .unwrap();
        let calls = seen.lock().unwrap();
        let (_, sent) = calls
            .iter()
            .rfind(|(name, _)| name == "chrome_execute")
            .unwrap();
        assert_eq!(sent["action"], call["action"]);
        let mut expected = call["args"].clone();
        if call["action"] == "page.read" && expected.get("cursor").is_none() {
            expected["maxBytes"] = json!(2000);
            expected["limit"] = json!(8);
        }
        assert_eq!(sent["args"], expected);
    }
    core.shutdown().await;
}
