//! Children use the built-in browser; tests the MCP catalog and refusal at authorization/hook boundaries.
use super::*;

#[tokio::test]
async fn a_child_names_the_builtin_and_main_chat_routes_even_with_a_mode() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let shared = &fixture.core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"child","parentSessionId":"parent"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    for profile in ["chrome:p", "edge:p"] {
        let browser = super::super::agent_browser::Browser::from_profile(profile);
        for tool in ["chrome_execute", "chrome_session", "chrome_status"] {
            shared
                .chrome_control
                .chat_profiles
                .lock()
                .unwrap()
                .insert("child".into(), profile.into());
            let error =
                authorize(shared, "child", tool, &json!({"action":"page.read"})).unwrap_err();
            assert_eq!(
                error,
                format!(
                    "Not run: subagents cannot use {}. Use Vibe Work's built-in browser (browser_read / browser_act) instead, or ask the main chat for steps in {}.",
                    browser.name(),
                    browser.name()
                )
            );
        }
    }
    // A remembered mode does not override the subagent restriction.
    shared.access.observe_request(
        "session/agent/update",
        "child",
        &json!({"agentName":"auto-approve"}),
    );
    assert!(
        authorize(
            shared,
            "child",
            "chrome_execute",
            &json!({"action":"page.read"})
        )
        .is_err()
    );
    fixture.finish().await;
}

#[tokio::test]
async fn a_main_chat_without_a_mode_is_not_mislabeled_as_a_subagent() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    let shared = &fixture.core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"main"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    assert_eq!(
        authorize(
            shared,
            "main",
            "chrome_execute",
            &json!({"action":"page.read"})
        )
        .unwrap_err(),
        "Not run: this chat's access mode is unavailable."
    );
    fixture.finish().await;
}

#[test]
fn the_session_mcp_catalog_contains_both_builtin_browser_tools() {
    let catalog = vw_agent_tools::mcp::tool_list();
    for name in [
        vw_agent_tools::wire::tools::BROWSER_READ,
        vw_agent_tools::wire::tools::BROWSER_ACT,
    ] {
        assert!(
            catalog
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == name)
        );
    }
}

#[tokio::test]
async fn a_child_hook_cannot_borrow_the_main_chats_browser_authority() {
    let core = super::super::output_tests::core("child-browser-hook").await;
    let shared = &core.shared;
    let _ = shared
        .store
        .load(
            json!({"session":{"id":"child","parentSessionId":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared.access.observe_request(
        "session/agent/update",
        "child",
        &json!({"agentName":"auto-approve"}),
    );
    for profile in ["chrome:p", "edge:p"] {
        shared
            .chrome_control
            .work
            .lock()
            .unwrap()
            .open("t", "s", "active", profile, None);
        for source in ["child", "unloaded-child"] {
            for tool in ["chrome_execute", "chrome_session", "chrome_status"] {
                let result = check(
                    shared,
                    &json!({"sessionId":source,"parentSessionId":"s",
                    "toolName":format!("mcp_vw.{tool}"),"toolInput":{"sessionId":"t","action":"page.read"}}),
                );
                assert_eq!(result["decision"], "deny", "{source} {tool}: {result}");
                assert_eq!(
                    result["reason"],
                    format!(
                        "Not run: subagents cannot use {}. Use Vibe Work's built-in browser (browser_read / browser_act) instead, or ask the main chat for steps in {}.",
                        super::super::agent_browser::Browser::from_profile(profile).name(),
                        super::super::agent_browser::Browser::from_profile(profile).name()
                    )
                );
            }
        }
    }
    let main = check(
        shared,
        &json!({"sessionId":"s","parentSessionId":"s",
        "toolName":"mcp_vw.chrome_execute","toolInput":{"sessionId":"t","action":"page.read"}}),
    );
    assert_eq!(main["decision"], "allow");
    core.shutdown().await;
}
