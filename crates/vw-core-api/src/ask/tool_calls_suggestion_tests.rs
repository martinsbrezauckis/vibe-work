//! Regression tests for `tool_calls.rs`; uses the parent module and is called by Cargo tests.
use super::*;
#[tokio::test]
async fn addon_cards_need_no_approval_in_any_access_mode() {
    let core = crate::offline_core::start("addon-card-permission").await;
    for mode in [
        "ask",
        "accept-edits",
        "smart-approve",
        "auto-approve",
        "vw-plan",
        "plan",
        "explore",
    ] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for tool in [names::CONNECTOR_SUGGEST, names::ADDON_SUGGEST] {
            let answer = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                permission(
                    &core.shared,
                    "s",
                    "s",
                    "call",
                    tool,
                    &json!({"addon":"chrome","reason":"Read a page"}),
                ),
            )
            .await;
            assert!(matches!(answer, Ok(Ok(()))), "{mode} {tool}: {answer:?}");
            assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
        }
    }
    core.shutdown().await;
}
