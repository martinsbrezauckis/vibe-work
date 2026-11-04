//! Connected extension schemas through app overrides; fixtures cover mixed browser versions and disconnects.
use super::{capabilities, gateway, output_tests, test_support};
use serde_json::{Value, json};

fn mixed_status(args: &Value) -> Value {
    let mut result = output_tests::status(args);
    let body = &mut result["structuredContent"];
    body["profiles"][0]["extensionCapabilities"] = json!([]);
    body["profiles"][0]["extensionVersion"] = json!("0.1.4");
    body["profiles"][1]["extensionCapabilities"] = json!([
        "page.read.includeText",
        "page.read.cursor",
        "page.read.scope.viewport",
        "page.read.includeBounds",
        "page.observation.changes",
        "page.key.frameId"
    ]);
    for row in body["capabilities"]["actions"].as_array_mut().unwrap() {
        if let Some(schema) = row.get("inputSchema").cloned() {
            let mut legacy = schema.clone();
            legacy["properties"]
                .as_object_mut()
                .unwrap()
                .retain(|key, _| {
                    ![
                        "includeText",
                        "cursor",
                        "scope",
                        "includeBounds",
                        "observation",
                    ]
                    .contains(&key.as_str())
                });
            row["inputSchema"] = legacy.clone();
            row["profileSchemas"] = json!({"chrome:profile-0":legacy,"chrome:profile-1":schema});
        }
    }
    result
}

#[tokio::test]
async fn lookup_follows_selected_profile_and_admission_follows_exact_session() {
    let core = output_tests::core("chrome-connected-schema").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            mixed_status(args)
        } else {
            output_tests::mcp(json!({"status":"completed"}))
        }
    });
    for (profile, supported) in [("chrome:profile-0", false), ("chrome:profile-1", true)] {
        core.shared
            .chrome_control
            .chat_profiles
            .lock()
            .unwrap()
            .insert("s".into(), profile.into());
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_status",
            json!({"action":"page.read"}),
        )
        .await
        .unwrap();
        let row = &reply["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
        assert_eq!(
            output_tests::schema(row)["properties"]
                .get("includeText")
                .is_some(),
            supported
        );
        assert!(row.get("profileSchemas").is_none());
    }
    for (session, supported) in [("t", false), ("t2", true)] {
        let reply = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":session,"action":"page.read","args":{"includeText":false}}),
        )
        .await;
        assert_eq!(reply.is_ok(), supported, "{reply:?}");
    }
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "chrome_execute")
            .count(),
        1
    );
    // Find and returned helper cursors remain usable with a legacy extension.
    for (action, args) in [
        ("page.find", json!({"query":"Search"})),
        ("page.read", json!({"cursor":"h-returned-token"})),
    ] {
        gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":action,"args":args}),
        )
        .await
        .unwrap();
    }
    core.shutdown().await;
}

#[test]
fn disconnected_profile_does_not_advertise_remembered_options() {
    let mut result = output_tests::status(&json!({"action":"page.click"}));
    result["structuredContent"]["profiles"][0]["connected"] = json!(false);
    capabilities::filter(&mut result, Some("chrome:profile-0"));
    assert!(
        result["structuredContent"]["capabilities"]["actions"][0]["inputSchema"]["properties"]
            .get("observation")
            .is_none()
    );
    let mut overview = output_tests::status(&json!({}));
    capabilities::filter(&mut overview, None);
    assert!(
        overview["structuredContent"]["capabilities"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("inputSchema").is_none())
    );
}

#[test]
fn connected_profile_filters_optional_observation_arguments() {
    for selected in [None, Some("chrome:profile-0"), Some("chrome:profile-1")] {
        let mut result = output_tests::status(&json!({"action":"page.read"}));
        let profiles = result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap();
        profiles[0]["extensionCapabilities"] = json!([]);
        profiles[0]["extensionVersion"] = json!("0.1.4");
        profiles[1]["extensionCapabilities"] = json!([
            "page.read.includeText",
            "page.read.cursor",
            "page.read.scope.viewport",
            "page.read.includeBounds"
        ]);
        capabilities::filter(&mut result, selected);
        let props =
            &result["structuredContent"]["capabilities"]["actions"][0]["inputSchema"]["properties"];
        assert_eq!(
            props.get("includeText").is_some(),
            selected == Some("chrome:profile-1")
        );
        assert!(props.get("interactiveOnly").is_some());
        assert!(props.get("offset").is_some());
    }
}

#[test]
fn legacy_key_description_does_not_advertise_frame_argument() {
    let mut result = output_tests::status(&json!({"action":"page.key"}));
    capabilities::filter(&mut result, Some("chrome:profile-0"));
    let row = &result["structuredContent"]["capabilities"]["actions"][0];
    assert!(row["inputSchema"]["properties"].get("frameId").is_none());
    assert!(!row["description"].as_str().unwrap().contains("frameId"));
}
