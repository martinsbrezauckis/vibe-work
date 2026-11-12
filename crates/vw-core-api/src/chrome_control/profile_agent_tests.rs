//! Agent-visible profile identity, readable choices and missing-profile recovery through the real gateway.
use super::{edge_agent_tests::fixture, gateway, test_support};
use serde_json::{Value, json};
fn profiles() -> Value {
    json!([
        {"profileId":"chrome:work","name":"Work","connected":true},
        {"profileId":"chrome:personal","name":"Personal","connected":true},
        {"profileId":"edge:work","name":"Work","connected":true}
    ])
}
#[allow(
    clippy::unwrap_used,
    reason = "person selection is required test setup"
)]
async fn selected(name: &str) -> crate::Core {
    let core = fixture(name, profiles()).await;
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::SelectProfile {
            session_id: "s".into(),
            profile_id: "chrome:work".into(),
        },
    )
    .await
    .unwrap();
    core
}
#[tokio::test]
async fn status_exposes_the_fixed_selection_and_every_named_choice() {
    let core = selected("profile-agent-status").await;
    let result = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    let body = &result["vwMcpResult"]["structuredContent"];
    assert_eq!(body["selectedProfile"]["profileId"], "chrome:work");
    assert_eq!(body["selectedProfile"]["name"], "Work");
    assert_eq!(body["profileChoices"].as_array().unwrap().len(), 3);
    assert_eq!(body["selectedProfile"]["state"], "connected");
    core.shutdown().await;
}
#[tokio::test]
async fn open_and_execute_receipts_keep_the_actual_profile_id_and_name() {
    let core = selected("profile-agent-receipt").await;
    for (tool, args) in [
        (
            "chrome_session",
            json!({"action":"open","profileId":"chrome"}),
        ),
        (
            "chrome_execute",
            json!({"sessionId":"task","action":"page.read","args":{}}),
        ),
    ] {
        let result = gateway::call(&core.shared, "s", tool, args).await.unwrap();
        let body = &result["vwMcpResult"]["structuredContent"];
        assert_eq!(body["profileId"], "chrome:work");
        assert_eq!(body["profileName"], "Work");
    }
    core.shutdown().await;
}
#[tokio::test]
async fn a_missing_bound_profile_names_it_and_never_dispatches_to_the_other_account() {
    let core = selected("profile-agent-missing").await;
    let status = json!({"structuredContent":{"profiles":[{"profileId":"chrome:work","name":"Work","connected":false},{"profileId":"chrome:personal","name":"Personal","connected":true}]}});
    super::owner::update_pairing(&std::sync::Arc::downgrade(&core.shared), &status);
    let seen = test_support::answering(&core.shared, move |_, _| status.clone());
    let result = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"chrome"}),
    )
    .await;
    let text = match result {
        Ok(value) => value.to_string(),
        Err(error) => error,
    };
    assert!(text.contains("Chrome · Work is unavailable; no action ran. Select a connected profile or reconnect this profile."), "{text}");
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_session")
    );
    assert_eq!(
        super::profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some("chrome:work")
    );
    core.shutdown().await;
}
