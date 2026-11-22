//! Chrome tool argument and listing checks that run before any helper call.
use serde_json::json;

#[tokio::test]
async fn closed_chat_browser_calls_explain_how_to_resume() {
    let core = crate::offline_core::start("browser-closed-chat").await;
    let shared = &core.shared;
    let reason = super::call_selected(shared, "missing", "chrome_status", json!({}))
        .await
        .unwrap_err();
    assert!(reason.contains("Open the chat in Vibe Work"), "{reason}");
    let reason =
        super::local_reply(shared, "missing", "chrome_status", &json!({}), None).unwrap_err();
    assert!(reason.contains("Open the chat in Vibe Work"), "{reason}");
    core.shutdown().await;
}

#[tokio::test]
async fn closed_chat_permission_check_explains_how_to_resume() {
    let core = crate::offline_core::start("browser-closed-permission").await;
    let reason =
        super::super::check::authorize(&core.shared, "missing", "chrome_status", &json!({}))
            .unwrap_err();
    assert!(reason.contains("Open the chat in Vibe Work"), "{reason}");
    core.shutdown().await;
}

#[tokio::test]
async fn missing_chrome_session_fields_are_named_before_any_helper_call() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    for (args, field) in [
        (json!({}), "action"),
        (json!({"action":"open"}), "profileId"),
        (json!({"action":"open","profileId":""}), "profileId"),
        (json!({"action":"get"}), "sessionId"),
        (json!({"action":"cancel"}), "sessionId"),
        (json!({"action":"stop","sessionId":""}), "sessionId"),
    ] {
        let reason = super::call(&fixture.core.shared, "s", "chrome_session", args)
            .await
            .unwrap_err();
        assert!(reason.contains(field), "missing {field}: {reason}");
    }
    fixture.finish().await;
}

#[tokio::test]
async fn malformed_execute_input_is_refused_before_reopening_a_chrome_session() {
    assert!(
        super::validate_execute_args(
            &json!({"sessionId":"t","action":"page.read","args":{}}),
            super::Browser::Chrome
        )
        .is_ok()
    );
    assert!(
        super::validate_execute_args(
            &json!({"sessionId":"t","steps":[{"action":"page.read","args":{}}]}),
            super::Browser::Chrome
        )
        .is_ok()
    );
    let core = crate::offline_core::start_with("chrome-input-shape", |config, dir| {
        config.chrome_control.package = Some(dir.join("package"));
        config.chrome_control.user_home = Some(dir.join("chrome-home"));
    })
    .await;
    for args in [
        json!({"sessionId":"unopened","action":"page.read"}),
        json!({"sessionId":"unopened","action":"page.read","args":null}),
        json!({"sessionId":"unopened","action":"page.read","args":{},"steps":[]}),
    ] {
        let reason = super::call(&core.shared, "s", "chrome_execute", args)
            .await
            .unwrap_err();
        assert!(
            reason.contains("action and args") || reason.contains("batch"),
            "{reason}"
        );
    }
    core.shutdown().await;
}

/// Vibe lists a session's MCP tools once, at session start, often before the core has the session's view or
/// mode, and keeps the answer for up to a day for every chat whose tool server has the same launch line.
/// Before the change the list was empty for a known Plan/Explore chat or with the Chrome
/// switch off, so such a chat seeded an empty Chrome list that later chats got from Vibe's cache. After it the
/// list depends only on the package; the launch line carries the chat's choice (`agent_tools::chrome_launch`).
#[tokio::test]
async fn the_tool_list_is_the_same_for_every_chat_and_switch_state() {
    let absent = crate::offline_core::start("chrome-listing-absent").await;
    assert_eq!(
        super::list(&absent.shared),
        json!([]),
        "no package: no Chrome tools"
    );
    absent.shutdown().await;
    let core = crate::offline_core::start_with("chrome-listing", |config, dir| {
        // Its own home: the enabled switch below is saved in the Chrome state folder, never the real home.
        config.chrome_control.package = Some(dir.join("package"));
        config.chrome_control.user_home = Some(dir.join("chrome-home"));
    })
    .await;
    let shared = &core.shared;
    let full = json!(vw_agent_tools::chrome_tool_list());
    assert_ne!(full, json!([]));
    assert_eq!(super::list(shared), full, "a session still starting");
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    for enabled in [true, false] {
        shared.chrome_control.set_enabled(enabled).unwrap();
        shared
            .chrome_control
            .set_browser_enabled(super::super::agent_browser::Browser::Edge, enabled)
            .unwrap();
        for mode in ["ask", "auto-approve", "future-policy"] {
            shared.access.observe_request(
                "session/approvalPolicy/update",
                "s",
                &json!({"approvalPolicy":mode}),
            );
            assert_eq!(super::list(shared), full, "{mode} {enabled}");
            let refused = super::super::check::authorize(shared, "s", "chrome_status", &json!({}));
            assert_eq!(
                refused.is_err(),
                !enabled || mode == "future-policy",
                "{mode} {enabled}: {refused:?}"
            );
        }
    }
    core.shutdown().await;
}
