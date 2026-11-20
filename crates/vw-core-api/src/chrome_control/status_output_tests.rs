//! Compact `chrome_status` shape: per-mode action rows, shared feature lists, profile defaults and full session records within the return budget.
//! Shipped-helper-shaped fixtures; no browser or helper process.
use super::output_tests::{catalog, core, fits, status};
use super::{gateway, test_support};
use serde_json::{Value, json};

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
        assert_eq!(actions.as_object().unwrap().len(), 85);
        let capabilities = &status["capabilities"];
        assert!(action_row(capabilities, "page.read").get("needs").is_none());
        assert!(action_row(capabilities, "page.type").get("needs").is_none());
        assert_eq!(
            action_row(capabilities, "page.type")["approvalRequired"] == true,
            mode == "ask"
        );
        assert_eq!(
            action_row(capabilities, "raw.execute")
                .get("needs")
                .is_some(),
            mode != "auto-approve"
        );
        assert_eq!(
            action_row(capabilities, "cookies.read")
                .get("needs")
                .is_some(),
            mode != "auto-approve"
        );
    }
    core.shutdown().await;
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
    // 0.1.10: the merged browser actions (input, Firefox, Safari) put the compacted status at 6,160 characters.
    fits("status with full session records", &answer, 6400);
    let status = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
    assert_eq!(status["sessionDefaults"]["clientSessionId"], "s");
    assert_eq!(status["sessions"].as_array().unwrap().len(), 2);
    let capabilities = &status["capabilities"];
    assert_catalog_features(status);
    assert_ask_authority(capabilities);
    for action in ["history.read", "bookmarks.read"] {
        assert!(
            action_row(capabilities, action)
                .get("approvalRequired")
                .is_none()
        );
        assert!(action_row(capabilities, action).get("needs").is_none());
    }
    for action in catalog()["actions"].as_array().unwrap() {
        let id = action["id"].as_str().unwrap();
        let name = &action_row(capabilities, id)["capability"];
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
        // Duplicate state labels and unknown versions can be omitted; every known fact stays exact.
        assert_eq!(expected["label"], expected["state"]);
        expected.as_object_mut().unwrap().remove("label");
        if expected["extensionVersion"].is_null() {
            expected.as_object_mut().unwrap().remove("extensionVersion");
        }
        if let Some(sharing) = expected.get_mut("sharing").and_then(Value::as_object_mut)
            && sharing.get("extensionVersion").is_some_and(Value::is_null)
        {
            sharing.remove("extensionVersion");
        }
        assert_eq!(actual, &expected);
    }
    if cfg!(feature = "your-edge") {
        assert_eq!(addons[1]["stateCode"], "notSetUp");
        assert!(capabilities["capabilityNames"].is_array());
    }
    if status["profileDefaults"].is_object() {
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
    assert_catalog_features(status);
    let partial = lists[1].as_array().unwrap();
    assert_eq!(
        partial.len(),
        catalog()["actions"].as_array().unwrap().len() - 1
    );
    assert!(!partial.contains(&json!("raw.execute")));
    for action in catalog()["actions"].as_array().unwrap() {
        assert_eq!(
            partial.contains(&action["id"]),
            action["id"] != "raw.execute"
        );
    }
    core.shutdown().await;
}

fn action_row<'a>(capabilities: &'a Value, id: &str) -> &'a Value {
    let row = &capabilities["actions"][id];
    if let Some(index) = row.as_u64() {
        assert!(
            capabilities["availability"]
                .as_str()
                .unwrap()
                .contains("actionRows")
        );
        &capabilities["actionRows"][usize::try_from(index).unwrap()]
    } else {
        row
    }
}

fn assert_ask_authority(capabilities: &Value) {
    assert_eq!(
        action_row(capabilities, "page.type")["approvalRequired"],
        true
    );
    assert_eq!(
        action_row(capabilities, "page.key")["typingKeysApprovalRequired"],
        true
    );
    for action in ["raw.execute", "cookies.read", "clipboard.read_rich"] {
        let row = action_row(capabilities, action);
        let mode = usize::try_from(row["needs"].as_u64().unwrap()).unwrap();
        assert_eq!(
            capabilities["modes"][mode],
            vw_ask::chrome::Tier::FullAccess.label()
        );
        assert!(row.get("approvalRequired").is_none(), "{action}: {row}");
    }
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

/// PREDICT (written before the status tables were kept whole): an over-budget status halves its largest array, the
/// shared `featureLists` table, so a profile's `{"list":1}` points at nothing and the agent reads "no capabilities".
#[tokio::test]
async fn an_over_budget_status_keeps_every_feature_list_its_profiles_name() {
    let core = core("chrome-output-feature-lists-budget").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = status(args);
        let names: Vec<Value> = (0..200)
            .map(|i| json!(format!("extension.capability.{i:03}.v1")))
            .collect();
        for profile in result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap()
        {
            profile["extensionCapabilities"] = json!(names);
        }
        result
    });
    for args in [json!({}), json!({"profileId":"chrome:profile-0"})] {
        let answer = gateway::call(&core.shared, "s", "chrome_status", args.clone())
            .await
            .unwrap();
        let status = &answer["vwMcpResult"]["structuredContent"]["untrustedPageContent"];
        let profiles = status["profiles"].as_array().unwrap();
        assert_eq!(
            profiles.len(),
            2,
            "{args}: connected profiles must survive capabilities"
        );
        let defaults = status["profileDefaults"].clone();
        for profile in profiles {
            let mut restored = defaults.as_object().cloned().unwrap_or_default();
            restored.extend(profile.as_object().unwrap().clone());
            assert_eq!(restored["connected"], true, "{args}: {restored:?}");
            assert_eq!(
                restored["extensionVersion"], "0.1.6",
                "{args}: {restored:?}"
            );
            assert_eq!(
                restored["browserVersion"], "142.0.7444.60",
                "{args}: {restored:?}"
            );
        }
        if let Some(lists) = status["featureLists"].as_array() {
            for profile in status["profiles"].as_array().unwrap() {
                for key in ["features", "extensionCapabilities"] {
                    let reference = profile.get(key).unwrap_or(&defaults[key]);
                    let index = usize::try_from(reference["list"].as_u64().unwrap()).unwrap();
                    assert!(
                        index < lists.len(),
                        "{args}: {key} -> {index} of {}",
                        lists.len()
                    );
                }
            }
            let text = answer["vwMcpResult"]["content"].to_string();
            assert!(
                text.contains("extension.capability.199.v1"),
                "{args}: list cut"
            );
        } else {
            assert!(
                status["capabilities"]["availability"]
                    .as_str()
                    .is_some_and(|s| s.contains("omitted") && s.contains("chrome_status"))
            );
            for profile in status["profiles"].as_array().unwrap() {
                for key in ["features", "extensionCapabilities"] {
                    assert!(profile.get(key).is_none() && defaults.get(key).is_none());
                }
            }
        }
        let body = &answer["vwMcpResult"]["structuredContent"];
        assert!(body.to_string().len() <= 9000);
        assert!(serde_json::to_string_pretty(body).unwrap().len() <= 15_000);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn crowded_status_keeps_profiles_and_versions_within_one_return() {
    let core = core("chrome-output-crowded-capabilities").await;
    test_support::answering(&core.shared, |_, args| {
        let mut result = status(args);
        let names: Vec<Value> = (0..512)
            .map(|i| json!(format!("cap{i:03}{}", "x".repeat(122))))
            .collect();
        for profile in result["structuredContent"]["profiles"]
            .as_array_mut()
            .unwrap()
        {
            profile["extensionCapabilities"] = json!(names);
        }
        result
    });
    let answer = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let body = &answer["vwMcpResult"]["structuredContent"];
    let status = &body["untrustedPageContent"];
    assert!(
        body["outputLimit"].as_str().unwrap().contains("9000"),
        "status reports its complete return budget: {}",
        body["outputLimit"]
    );
    assert!(
        body["outputLimit"]
            .as_str()
            .unwrap()
            .contains("chrome_status")
    );
    assert!(
        body.to_string().len() <= 9000,
        "compact status exceeded budget"
    );
    assert!(serde_json::to_string_pretty(body).unwrap().len() <= 15_000);
    assert_eq!(status["profiles"].as_array().map(Vec::len), Some(2));
    let defaults = status["profileDefaults"].clone();
    for profile in status["profiles"].as_array().unwrap() {
        let mut restored = defaults.as_object().cloned().unwrap_or_default();
        restored.extend(profile.as_object().unwrap().clone());
        assert_eq!(restored["connected"], true);
        assert_eq!(restored["browserVersion"], "142.0.7444.60");
        assert_eq!(restored["extensionVersion"], "0.1.6");
        assert!(restored.get("features").is_none());
        assert!(restored.get("extensionCapabilities").is_none());
    }
    assert!(status.get("featureLists").is_none());
    let availability = status["capabilities"]["availability"].as_str().unwrap();
    assert!(availability.contains("chrome_status") && availability.contains("profileId"));
    assert!(availability.contains("omitted"), "{availability}");
    core.shutdown().await;
}
