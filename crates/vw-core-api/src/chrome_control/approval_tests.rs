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
