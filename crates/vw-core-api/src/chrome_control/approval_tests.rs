//! Approval snapshots take browser identity from the action's helper session, including dual-browser chats.
use serde_json::json;

#[tokio::test]
async fn approval_browser_is_bound_to_the_action_session() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let shared = &fixture.core.shared;
    for profiles in [["chrome:p", "edge:p"], ["edge:p", "chrome:p"]] {
        for profile in profiles {
            shared
                .chrome_control
                .work
                .lock()
                .unwrap()
                .open(profile, "chat", "active", profile, None);
            shared
                .chrome_control
                .chat_profiles
                .lock()
                .unwrap()
                .insert("chat".into(), profile.into());
        }
        for profile in profiles {
            let other = if profile == "chrome:p" {
                "edge:p"
            } else {
                "chrome:p"
            };
            let entry = json!({"id":"approval", "type":"callback", "sessionId":"chat", "callbackId":"approval",
                "vwBrowser":"forged", "detail":{"kind":"approval", "effect":{"kind":"tool", "toolName":"mcp_vw.chrome_execute",
                "input":{"sessionId":profile,"profileId":other,"action":"history.read","args":{}}}}});
            let _ = shared
                .store
                .load(
                    json!({"session":{"id":"chat"},"history":[entry],"turns":[]}),
                    vw_session::LoadKind::New,
                )
                .unwrap();
            let snapshot = crate::sessions::snapshot(shared, "chat").unwrap();
            assert_eq!(
                snapshot.state["history"][0]["vwBrowser"],
                super::agent_browser::Browser::from_profile(profile).name()
            );
        }
    }
    fixture.finish().await;
}

#[tokio::test]
async fn click_card_uses_only_the_helpers_observed_ref_name() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let shared = &fixture.core.shared;
    let _ = super::output_cache::for_model(
        shared,
        "chat",
        &json!({"sessionId":"helper","action":"page.read"}),
        super::owner::Answer::Helper(json!({"structuredContent":{
            "sessionId":"helper","status":"completed","action":"page.read","data":{"refs":[
                {"ref":"accept","role":"button","name":"Accept all"},
                {"ref":"reject","role":"button","name":"Reject"}
            ]}
        }})),
        "chrome_execute",
        false,
    );
    for (reference, expected) in [
        ("accept", "Accept all"),
        ("reject", "Reject"),
        ("missing", ""),
    ] {
        let mut entry = json!({"sessionId":"chat","detail":{"effect":{
            "toolName":"mcp_vw.chrome_execute","input":{"sessionId":"helper","action":"page.click",
                "args":{"ref":reference,"name":"Forged approval name"}}
        }}});
        super::approval::project(shared, &mut entry);
        assert_eq!(
            entry["vwClickTarget"]["name"].as_str().unwrap_or(""),
            expected
        );
    }
    let mut other = json!({"sessionId":"other","detail":{"effect":{
        "toolName":"mcp_vw.chrome_execute","input":{"sessionId":"helper",
            "action":"page.click","args":{"ref":"accept"}}
    }}});
    super::approval::project(shared, &mut other);
    assert!(other["vwClickTarget"].is_null());
    fixture.finish().await;
}
