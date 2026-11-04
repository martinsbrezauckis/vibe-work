//! Optional summary/viewport discovery and admission require both helper schema and live profile support.
//! Exercises current and older helpers through the gateway, including selection versus exact session binding.
use super::{gateway, output_tests, test_support};
use serde_json::{Value, json};

fn status(args: &Value, helper: bool, extension: bool, connected: bool) -> Value {
    let mut result = output_tests::status(args);
    let body = &mut result["structuredContent"];
    for (index, profile) in body["profiles"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        profile["extensionCapabilities"] = if extension || index == 1 {
            json!(["page.read.summary", "page.find.scope.viewport"])
        } else {
            json!([])
        };
        if index == 0 {
            profile["connected"] = json!(connected);
        }
    }
    if !helper {
        for row in body["capabilities"]["actions"].as_array_mut().unwrap() {
            if let Some(fields) = row["inputSchema"]["properties"].as_object_mut() {
                fields.remove("summary");
                fields.remove("scope");
            }
            row["description"] = json!("Read controls; use returned refs.");
        }
    }
    result
}

#[tokio::test]
async fn summary_discovery_requires_the_helper_and_selected_live_profile() {
    let core = output_tests::core("chrome-summary-discovery").await;
    for (helper, extension, connected) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
        (true, true, true),
    ] {
        test_support::answering(&core.shared, move |_, args| {
            status(args, helper, extension, connected)
        });
        for selected in [None, Some("chrome:profile-0"), Some("chrome:profile-1")] {
            {
                let mut profiles = core.shared.chrome_control.chat_profiles.lock().unwrap();
                profiles.remove("s");
                if let Some(profile) = selected {
                    profiles.insert("s".into(), profile.into());
                }
            }
            let supported = helper
                && (selected == Some("chrome:profile-1")
                    || (selected.is_none() && !connected)
                    || (extension && connected));
            for action in ["page.read", "page.find"] {
                let reply =
                    gateway::call(&core.shared, "s", "chrome_status", json!({"action":action}))
                        .await
                        .unwrap();
                let row = &reply["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
                if row["available"] == false {
                    assert!(
                        !connected && selected == Some("chrome:profile-0") && action == "page.find"
                    );
                    assert!(!supported);
                    continue;
                }
                let schema = output_tests::schema(row);
                assert_eq!(
                    schema["properties"].get("summary").is_some(),
                    supported,
                    "{helper}/{extension}/{connected}/{selected:?}/{action}"
                );
                if action == "page.find" {
                    assert_eq!(schema["properties"].get("scope").is_some(), supported);
                }
                if !supported {
                    assert!(
                        !row["description"]
                            .as_str()
                            .unwrap()
                            .contains("summary:true")
                    );
                }
            }
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn summary_admission_rechecks_the_exact_session_before_dispatch() {
    let core = output_tests::core("chrome-summary-admission").await;
    core.shared
        .chrome_control
        .chat_profiles
        .lock()
        .unwrap()
        .insert("s".into(), "chrome:profile-1".into());
    for (helper, extension, connected) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
        (true, true, true),
    ] {
        let seen = test_support::answering(&core.shared, move |tool, args| {
            if tool == "chrome_status" {
                status(args, helper, extension, connected)
            } else {
                output_tests::mcp(json!({"status":"completed"}))
            }
        });
        let supported = helper && extension && connected;
        for (action, args) in [
            ("page.read", json!({"summary":true})),
            ("page.read", json!({"summary":false})),
            (
                "page.read",
                json!({"cursor":"h-returned-token","summary":true}),
            ),
            ("page.find", json!({"query":"Search","summary":true})),
            ("page.find", json!({"query":"Search","scope":"viewport"})),
        ] {
            let reply = gateway::call(
                &core.shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t","action":action,"args":args}),
            )
            .await;
            assert_eq!(
                reply.is_ok(),
                supported,
                "{helper}/{extension}/{connected}/{action}: {reply:?}"
            );
        }
        assert_eq!(
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(tool, _)| tool == "chrome_execute")
                .count(),
            if supported { 5 } else { 0 }
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn summary_support_in_another_profile_cannot_replace_the_session_binding() {
    let core = output_tests::core("chrome-summary-missing-binding").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            let mut reply = status(args, true, true, true);
            reply["structuredContent"]["sessions"] = json!([]);
            reply
        } else {
            output_tests::mcp(json!({"status":"completed"}))
        }
    });
    let reply = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{"summary":true}}),
    )
    .await;
    assert!(reply.is_err(), "missing helper session binding: {reply:?}");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    core.shutdown().await;
}
