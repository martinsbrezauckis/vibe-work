//! Extension diagnostics read in every mode; notifications send only after the shared write decision.
use serde_json::json;
use vw_ask::chrome::Tier;
#[test]
fn diagnostics_read_but_named_notifications_require_a_write_decision() {
    for action in ["extensions.list", "extensions.status", "notifications.list"] {
        assert_eq!(
            super::mapping::ask_required("chrome_execute", &json!({"action":action})),
            Some(Tier::Ask),
            "{action}"
        );
    }
    for action in ["notifications.create", "notifications.clear"] {
        assert_eq!(
            super::mapping::ask_required("chrome_execute", &json!({"action":action})),
            Some(Tier::AcceptEdits),
            "{action}"
        );
    }
}

#[test]
fn receipts_name_the_notification_only_after_readback_and_expose_missing_grants() {
    let args = json!({"tabId":7,"id":"saved","title":"Saved","message":"Page saved"});
    let steps = super::extension_state::steps("notifications.create", &args).unwrap();
    assert_eq!(steps[0]["method"], "notifications.create");
    assert_eq!(steps[1]["method"], "notifications.getAll");
    let result = super::extension_state::readback(
        "notifications.create",
        &args,
        &json!({"state":{"ids":["saved"]}}),
    )
    .unwrap();
    assert_eq!(result["id"], "saved");
    assert_eq!(result["title"], "Saved");
    assert!(
        super::extension_state::readback(
            "notifications.create",
            &args,
            &json!({"state":{"ids":[]}})
        )
        .is_err()
    );
    let result = super::extension_state::readback(
        "extensions.status",
        &json!({}),
        &json!({"state":{"granted":["tabs"],"fileAccess":false}}),
    )
    .unwrap();
    assert_eq!(result["permissionState"], "missing_permissions");
    assert!(
        result["missingPermissions"]
            .as_array()
            .unwrap()
            .contains(&json!("management"))
    );
    assert!(
        result["fileAccessInstruction"]
            .as_str()
            .unwrap()
            .contains("Allow access to file URLs")
    );
    assert!(
        super::extension_state::steps(
            "notifications.create",
            &json!({"tabId":7,"id":"../other","title":"a","message":"b"})
        )
        .is_err()
    );
    assert!(
        super::extension_state::steps(
            "extensions.list",
            &json!({"tabId":7,"method":"management.uninstall"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn permission_inventory_plan_carries_its_checked_read_capability() {
    let core = super::read_consent_tests::core("extension-permission-plan").await;
    super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
        }
        "owner.primitive" => {
            assert_eq!(args["steps"][0]["method"], "permissions.status");
            assert!(
                args["approvedCapabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("page.read")),
                "{args}"
            );
            json!({"status":"completed","data":{"results":{"state":{"granted":["tabs"],"fileAccess":false}}}})
        }
        _ => panic!("unexpected {tool}"),
    });
    let result = super::extension_state::send(
        &core.shared,
        "s",
        &json!({"sessionId":"t","action":"extensions.status","args":{"tabId":7}}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.value()["structuredContent"]["status"], "completed");
    core.shutdown().await;
}
