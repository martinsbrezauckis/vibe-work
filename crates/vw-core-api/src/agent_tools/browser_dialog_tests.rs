//! App-modal refusals are shared by built-in and connected browser action gateways.
//! Offline core only; a named modal prevents any browser dispatch.
#[tokio::test]
async fn browser_actions_name_the_app_modal_before_any_browser_dispatch() {
    let core = crate::offline_core::start_with("browser-modal", |_, _| {}).await;
    core.shared
        .browser_dialogs
        .set_ui(Some("Choose a folder".into()));
    let builtin = super::browser_tools::call(
        &core.shared,
        "s",
        "browser_act",
        serde_json::json!({"action":"click","ref":"r"}),
    )
    .await;
    let connected = crate::chrome_control::gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        serde_json::json!({"sessionId":"t","action":"page.click","args":{"ref":"r"}}),
    )
    .await;
    for result in [builtin, connected] {
        let reason = result.unwrap_err();
        assert!(reason.contains("APP_DIALOG_OPEN"), "{reason}");
        assert!(reason.contains("Choose a folder"), "{reason}");
        assert!(
            reason.contains("finish or close") && reason.contains("retry"),
            "{reason}"
        );
    }
    core.shutdown().await;
}
