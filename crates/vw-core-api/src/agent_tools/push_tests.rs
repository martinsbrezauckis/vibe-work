//! App-owned push acceptance using disposable local repositories and the chat's real card service.
//! PREDICT: disabled developer permissions refuse before any push or approval card.

use serde_json::json;

#[tokio::test]
async fn push_refuses_when_developer_permissions_are_off() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    fixture.load("chat", &fixture.root.join("project"), &[], true);
    let shared = &fixture.core.shared;
    let tools = shared.agent_tools.as_ref().unwrap();
    let result = super::tools::call_prepared(shared, tools, "chat", "push", json!({})).await;
    assert!(result.unwrap_err().contains("Developer permissions"));
    assert_eq!(fixture.core.asks(), [] as [vw_ask::Ask; 0]);
    fixture.finish().await;
}
