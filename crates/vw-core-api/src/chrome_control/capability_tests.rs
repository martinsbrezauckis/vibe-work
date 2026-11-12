//! Connected-extension discovery tests for the gateway and action tier table; no browser or UI required.
use serde_json::json;
use vw_ask::chrome::Tier;

const ACTIONS: &[&str] = &[
    "clipboard.read",
    "clipboard.write",
    "network.intercept",
    "tabs.emulate",
    "files.wait_download",
];

#[test]
fn window_close_uses_the_same_cumulative_browsing_tier_in_every_browser() {
    let args = json!({"action":"window.close","args":{}});
    assert_eq!(
        super::mapping::required("chrome_execute", &args),
        Some(Tier::Ask)
    );
    assert_eq!(
        super::builtin::required(&json!({"action":"window.close"})),
        Some(Tier::Ask)
    );
    for mode in ["ask", "accept-edits", "auto-approve"] {
        assert_eq!(
            super::mapping::approval_tier(Some(mode), "chrome_execute", &args),
            None
        );
    }
    for profile in ["chrome:p", "edge:p", "safari:p", "firefox:agent"] {
        let mut data = json!({"profileId":profile,"connected":true,"features":["window.close"]});
        assert!(super::capabilities::supports(&data, "window.close"));
        data["features"] = json!([]);
        assert!(!super::capabilities::supports(&data, "window.close"));
    }
}

#[test]
fn clipboard_readback_requires_the_mode_itself_to_be_full() {
    for action in [
        "clipboard.write_rich",
        "clipboard.read_rich",
        "clipboard.write",
    ] {
        let args = json!({"action":action,"args":{"mime":"text/html","text":"<p>Note</p>"}});
        for mode in ["ask", "accept-edits"] {
            assert!(
                super::mapping::check_full(Some(mode), "chrome_execute", &args).is_err(),
                "{mode} {action} returns actual clipboard content"
            );
            assert_eq!(
                super::mapping::approval_tier(Some(mode), "chrome_execute", &args),
                None
            );
        }
        assert!(super::mapping::check_full(Some("auto-approve"), "chrome_execute", &args).is_ok());
    }
}

#[test]
fn group_titles_use_the_shared_data_entry_tier() {
    for action in ["tabs.group", "tabs.group_update"] {
        let args = json!({"action":action,"args":{"title":"Research"}});
        assert_eq!(
            super::mapping::approval_tier(Some("ask"), "chrome_execute", &args),
            Some(Tier::AcceptEdits),
            "{action}"
        );
        for mode in ["accept-edits", "auto-approve"] {
            assert_eq!(
                super::mapping::approval_tier(Some(mode), "chrome_execute", &args),
                None,
                "{mode} {action}"
            );
        }
    }
    for args in [json!({}), json!({"groupId":12})] {
        assert_eq!(
            super::mapping::approval_tier(
                Some("ask"),
                "chrome_execute",
                &json!({"action":"tabs.group","args":args})
            ),
            None
        );
    }
}

#[test]
fn open_tab_context_is_a_shared_read_and_requires_live_helper_support() {
    assert_eq!(
        super::mapping::required(
            "chrome_execute",
            &json!({"action":"tabs.pin","args":{"pinned":true}})
        ),
        Some(Tier::Ask)
    );
    assert_eq!(
        super::mapping::required(
            "chrome_execute",
            &json!({"action":"tabs.context","args":{}})
        ),
        Some(Tier::Ask)
    );
    for profile in ["chrome:p", "edge:p", "safari:p", "firefox:agent"] {
        let mut data = json!({"profileId":profile,"connected":true,"features":["tabs.context"]});
        assert!(super::capabilities::supports(&data, "tabs.context"));
        data["features"] = json!([]);
        assert!(!super::capabilities::supports(&data, "tabs.context"));
    }
}

#[tokio::test]
async fn extension_actions_are_offered_only_by_a_connected_supporting_profile() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    for supported in [false, true] {
        let mut reply = json!({"structuredContent":{
            "profiles":[{"profileId":"p","connected":true,"features":ACTIONS,
                "extensionVersion":if supported {json!("0.1.6")} else {json!(null)},
                "extensionCapabilities":if supported {json!(ACTIONS)} else {json!([])}}],
            "capabilities":{"actions":ACTIONS.iter().map(|a|json!({"action":a})).collect::<Vec<_>>()}
        }});
        super::check::annotate_modes(&fixture.core.shared, "s", "chrome_status", &mut reply);
        let rows = reply["structuredContent"]["capabilities"]["actions"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), if supported { 5 } else { 0 }, "{reply}");
        assert_eq!(
            reply["structuredContent"]["profiles"][0]["features"]
                .as_array()
                .unwrap()
                .len(),
            rows.len()
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn execution_rechecks_support_for_this_session_and_keeps_old_permission_names() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let core = crate::offline_core::start_with("chrome-extension-support", |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    let live = Arc::new(AtomicBool::new(false));
    let source = live.clone();
    let seen = super::test_support::answering(shared, move |name, _| {
        if name == "chrome_status" {
            json!({"structuredContent":{"sessions":[{"sessionId":"t","profileId":"p"}],
                "profiles":[{"profileId":"p","connected":source.load(Ordering::SeqCst),
                    "extensionVersion":"0.1.6","features":ACTIONS,"extensionCapabilities":ACTIONS}]}})
        } else {
            json!({"structuredContent":{"status":"completed"}})
        }
    });
    for supported in [false, true, false] {
        live.store(supported, Ordering::SeqCst);
        for action in ACTIONS {
            let result = super::gateway::call(
                shared,
                "s",
                "chrome_execute",
                json!({"sessionId":"t","action":action,"args":{}}),
            )
            .await;
            assert_eq!(result.is_ok(), supported, "{action}: {result:?}");
            if let Err(reason) = result {
                assert!(
                    reason.contains("does not report") && reason.contains("chrome_status"),
                    "{reason}"
                );
            }
        }
    }
    super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    let sent = seen.lock().unwrap().clone();
    let old = sent
        .iter()
        .find(|(name, args)| name == "chrome_execute" && args["action"] == "page.read")
        .unwrap();
    for cap in [
        "clipboard.read",
        "clipboard.write",
        "network.intercept",
        "tabs.emulate",
    ] {
        assert!(
            !old.1["_approvedCapabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(cap))
        );
    }
    assert_eq!(
        sent.iter()
            .filter(|(name, args)| name == "chrome_execute" && args["action"] != "page.read")
            .count(),
        5
    );
    core.shutdown().await;
}

#[test]
fn disconnected_mixed_and_partial_profiles_never_imply_support() {
    let full = json!({"profileId":"new","connected":true,"extensionVersion":"0.1.6","extensionCapabilities":ACTIONS,"features":ACTIONS});
    for action in ACTIONS {
        assert!(super::capabilities::supports(&full, action));
        for field in [
            "connected",
            "extensionVersion",
            "extensionCapabilities",
            "features",
        ] {
            let mut partial = full.clone();
            partial.as_object_mut().unwrap().remove(field);
            assert!(
                !super::capabilities::supports(&partial, action),
                "{field}: {partial}"
            );
        }
    }
    let mut partial = full.clone();
    partial["extensionCapabilities"] = json!(["clipboard.write"]);
    assert!(super::capabilities::supports(&partial, "clipboard.write"));
    assert!(!super::capabilities::supports(&partial, "clipboard.read"));
}

#[test]
fn app_action_schemas_match_the_helper_catalog() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("action_catalog.json")).unwrap();
    assert_eq!(
        catalog["actions"].as_array().unwrap().len(),
        ACTIONS.len() + 5
    );
    if let Ok(path) = std::env::var("VW_CHROME_CATALOG") {
        let helper: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for row in catalog["actions"].as_array().unwrap() {
            let source = helper["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|h| h["id"] == row["id"]);
            let Some(source) = source else {
                assert_eq!(
                    row["id"], "page.find",
                    "only find may be absent in the old helper"
                );
                continue;
            };
            let mut schema = source["inputSchema"].clone();
            if row["id"] == "page.find" {
                schema["properties"]["limit"]["default"] = json!(5);
            }
            let mut expected = row["inputSchema"].clone();
            // Schema overrides must not invent arguments when the pinned helper still has the old contract.
            expected["properties"]
                .as_object_mut()
                .unwrap()
                .retain(|key, _| schema["properties"].get(key).is_some());
            for value in [&mut expected, &mut schema] {
                for property in value["properties"].as_object_mut().unwrap().values_mut() {
                    property.as_object_mut().unwrap().remove("description");
                }
            }
            assert_eq!(expected, schema, "{}", row["id"]);
            assert_eq!(row["capability"], source["capability"]);
        }
    }
}

#[test]
fn another_connected_profile_does_not_offer_actions_for_this_chats_old_extension() {
    let mut result = json!({"structuredContent":{
        "profiles":[{"profileId":"old","connected":true,"features":["page.read"]},
            {"profileId":"new","connected":true,"extensionVersion":"0.1.6","features":ACTIONS,"extensionCapabilities":ACTIONS}],
        "capabilities":{"actions":[{"action":"page.read"},{"action":"clipboard.read"}]}
    }});
    super::capabilities::filter(&mut result, Some("old"));
    assert_eq!(
        result["structuredContent"]["capabilities"]["actions"],
        json!([{"action":"page.read"}])
    );
    assert_eq!(
        result["structuredContent"]["profiles"][1]["features"],
        json!(ACTIONS)
    );
}

#[test]
fn helper_screenshot_options_survive_the_app_catalog_overlay() {
    let row = json!({"action":"capture.screenshot","inputSchema":{"type":"object","properties":{"format":{"enum":["png","jpeg"]},"labels":{"type":"boolean"}}}});
    let mut result = json!({"structuredContent":{"profiles":[],"capabilities":{"actions":[row]}}});
    super::capabilities::filter(&mut result, None);
    assert_eq!(
        result["structuredContent"]["capabilities"]["actions"][0]["inputSchema"]["properties"]["labels"]
            ["type"],
        "boolean"
    );
}

#[test]
fn rich_clipboard_read_and_write_are_full_in_the_shared_policy() {
    for action in ["clipboard.read_rich", "clipboard.write_rich"] {
        assert_eq!(
            super::mapping::required(
                "chrome_execute",
                &json!({"action":action,"args":{"mime":"text/html"}})
            ),
            Some(Tier::FullAccess),
            "{action}"
        );
    }
}
