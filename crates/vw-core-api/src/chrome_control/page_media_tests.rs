//! Named media actions retain cumulative modes without granting caller-supplied scripts at lower tiers.
use serde_json::json;
use vw_ask::chrome::Tier;
#[test]
fn media_uses_the_same_shared_read_tier_in_both_browsers() {
    for action in [
        "media.read",
        "media.play",
        "media.pause",
        "media.seek",
        "media.mute",
        "media.transcript",
    ] {
        let args = json!({"action":action});
        assert_eq!(
            super::mapping::ask_required("chrome_execute", &args),
            Some(Tier::Ask),
            "{action}"
        );
        assert_eq!(
            super::mapping::required("chrome_execute", &args),
            Some(Tier::Ask),
            "{action}"
        );
        assert_eq!(super::builtin::required(&args), Some(Tier::Ask), "{action}");
    }
    assert!(!super::mapping::capabilities(Tier::Ask).contains(&"raw.execute"));
    assert_eq!(
        super::mapping::required("chrome_execute", &json!({"action":"raw.execute"})),
        Some(Tier::FullAccess)
    );
}

#[tokio::test]
async fn named_media_compiles_only_fixed_code_and_waits_for_its_real_receipt() {
    let core = super::read_consent_tests::core("media-primitive-receipt").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
        }
        "owner.primitive" => {
            assert_eq!(args["approvedCapabilities"], json!(["raw.execute"]));
            assert_eq!(args["steps"][0]["world"], "agent");
            assert_eq!(args["tabId"], 7);
            json!({"operationId":"op","status":"queued"})
        }
        "owner.primitiveResult" => {
            json!({"operationId":"op","status":"completed","data":{"results":{"media":{"result":{"value":"{\"items\":[]}"}}}}})
        }
        _ => panic!("unexpected tool {tool}"),
    });
    let request = json!({"sessionId":"t","action":"media.read","args":{"tabId":7}});
    let answer =
        super::read_consent::send(&core.shared, "s", "chrome_execute", request.clone(), None)
            .await
            .unwrap();
    assert_eq!(
        answer.value()["structuredContent"]["data"]["items"],
        json!([])
    );
    assert_eq!(answer.value()["structuredContent"]["action"], "media.read");
    let mut injected = request;
    injected["args"]["expression"] = json!("document.cookie");
    assert!(
        super::read_consent::send(&core.shared, "s", "chrome_execute", injected, None)
            .await
            .is_err()
    );
    core.shutdown().await;
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "owner.primitive")
            .count(),
        1
    );
}

#[test]
fn named_media_discovery_requires_the_actual_live_transport() {
    for live in [false, true] {
        let mut status = json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":live,"extensionCapabilities":["transport.contract.v1"],"features":[]}],"capabilities":{"actions":[]}}});
        super::page_plan_catalog::decorate(&mut status, Some("media.seek"));
        let rows = status["structuredContent"]["capabilities"]["actions"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), usize::from(live));
        if live {
            assert_eq!(
                rows[0]["inputSchema"]["required"],
                json!(["tabId", "mediaId", "timeMs"])
            );
        }
    }
}

#[test]
fn firefox_page_discovery_advertises_media_and_captures_without_native_tab_apis() {
    let mut status = json!({"structuredContent":{"profiles":[{"profileId":"firefox:agent","connected":true,"extensionCapabilities":["firefox.page.v1"],"features":[]}],"capabilities":{"actions":[]}}});
    super::page_plan_catalog::decorate(&mut status, None);
    let features = status["structuredContent"]["profiles"][0]["features"]
        .as_array()
        .unwrap();
    assert!(features.contains(&json!("media.read")));
    assert!(features.contains(&json!("capture.page")));
    assert!(!features.contains(&json!("tabs.mute")));
    assert!(!features.contains(&json!("extensions.list")));
}
