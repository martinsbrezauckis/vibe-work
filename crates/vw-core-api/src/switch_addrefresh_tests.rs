//! Regression tests for immediate MCP refresh with a person-disabled command server.
//! Reuses the switch mock channels to check saved state, requests and next-work convergence.

use super::*;

const DISABLED: &str =
    "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true}]\n";

async fn save(core: &Core, update: bool) {
    let command = std::env::current_exe().unwrap();
    let request = serde_json::from_value(json!({
        "op": if update { "updateCommandServer" } else { "addCommandServer" },
        "name": "x", "command": command, "args": if update { vec!["changed"] } else { vec![] }, "env": []
    })).unwrap();
    core.execute(Origin::Ui, Command::Connectors { request })
        .await
        .unwrap();
}

#[tokio::test]
async fn add_and_change_refresh_immediately_in_two_chats() {
    let core = start_with_keys("addrefresh-two", None, true, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "fresh", 1, false, false, &calls, ok);
    chat(&core, "ran", 2, false, true, &calls, ok);
    for update in [false, true] {
        let before = count(&calls, None, "mcp_catalog/refresh");
        save(&core, update).await;
        assert_eq!(count(&calls, None, "mcp_catalog/refresh"), before + 2);
        for sid in ["fresh", "ran"] {
            assert!(!core.shared.channels.switch_stale(sid));
        }
    }
    assert_eq!(count(&calls, None, "mcp_catalog/toggle"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn shared_refresh_reaches_two_chats_without_switching_their_generation() {
    let config = "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true},{name='http',transport='http',url='http://127.0.0.1:9/mcp'}]\n";
    let core = start_with_keys("addrefresh-shared", None, true, config).await;
    let calls: Calls = Arc::default();
    chat(&core, "busy", 1, true, true, &calls, ok);
    chat(&core, "idle", 2, false, false, &calls, ok);
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 2);
    assert_eq!(count(&calls, None, "config/reload"), 0);
    assert!(core.shared.channels.has_channel("busy"));
    assert!(!core.shared.channels.switch_stale("busy"));
    assert!(!core.shared.channels.switch_stale("idle"));
    core.shutdown().await;
}

#[tokio::test]
async fn forced_admission_and_both_raw_aliases_admit_disabled_rows_while_on() {
    let core = start_with_keys("addrefresh-raw", None, true, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "idle", 1, false, false, &calls, ok);
    assert!(crate::command_pause::admit_internal_refresh(
        &core.shared,
        Some("idle")
    ));
    for method in ["mcp_catalog/refresh", "mcp/refresh"] {
        for origin in [Origin::Ui, Origin::DebugApi] {
            let result = core
                .execute(
                    origin,
                    Command::Call {
                        method: method.to_owned(),
                        params: json!({"sessionId": "idle"}),
                        confirmation_token: None,
                    },
                )
                .await;
            assert!(result.is_ok(), "{method} {origin:?}: {result:?}");
        }
    }
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 2);
    assert_eq!(count(&calls, None, "mcp/refresh"), 2);
    core.shutdown().await;
}

#[tokio::test]
async fn paused_command_refresh_works_while_permissions_are_off_but_enabled_rows_are_blocked() {
    let core = start_with_keys("refresh-off-paused", None, false, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "idle", 1, false, false, &calls, ok);
    for method in ["mcp_catalog/refresh", "mcp/refresh"] {
        core.execute(
            Origin::Ui,
            Command::Call {
                method: method.to_owned(),
                params: json!({"sessionId":"idle"}),
                confirmation_token: None,
            },
        )
        .await
        .expect("disabled command rows must permit refresh while OFF");
    }
    let path = core.shared.instructions.own_home.join("config.toml");
    std::fs::write(&path, DISABLED.replace("disabled=true", "disabled=false")).unwrap();
    assert!(!crate::command_pause::admit_internal_refresh(
        &core.shared,
        None
    ));
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 1);
    assert_eq!(count(&calls, None, "mcp/refresh"), 1);
    core.shutdown().await;
}

#[tokio::test]
async fn a_refresh_waiting_for_one_chat_does_not_hold_the_shared_mutation_lock() {
    let core = start_with_keys(
        "refresh-lock-order",
        None,
        true,
        "mcp_servers=[{name='x',transport='stdio',command='echo'}]",
    )
    .await;
    let calls: Calls = Arc::default();
    chat(&core, "idle", 1, false, false, &calls, ok);
    let chat_lock = core.shared.plugin_refresh.lock("idle").await;
    let started = Arc::new(tokio::sync::Notify::new());
    let notified = started.clone();
    let shared = core.shared.clone();
    let refresh = tokio::spawn(async move {
        notified.notify_one();
        Core { shared }
            .forward(
                Origin::Ui,
                "mcp_catalog/refresh",
                json!({"sessionId":"idle"}),
                None,
            )
            .await
    });
    started.notified().await;
    let mutation = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        core.shared.developer_permissions.mutation_lock(),
    )
    .await
    .expect("a refresh waiting for a chat must not block the switch or another chat's mutation");
    // Recheck authority after the chat wait: an enabled row is unsafe if OFF was published meanwhile.
    core.shared.developer_permissions.set_for_test(false);
    drop(mutation);
    drop(chat_lock);
    let error = refresh.await.unwrap().unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::Denied);
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn forced_admission_checks_both_forms_and_empty_missing_malformed_inputs() {
    let core = start_with_keys("addrefresh-inputs", None, true, "").await;
    let path = core.shared.instructions.own_home.join("config.toml");
    for (text, expected) in [
        (DISABLED, true),
        (
            "[[mcp_servers]]\nname='y'\ntransport='stdio'\ncommand='echo'\ndisabled=true\n",
            true,
        ),
        (
            "mcp_servers = [{name='x',transport='stdio',command='echo'}]\n",
            true,
        ),
        (
            "mcp_servers = [{name='http',transport='http',disabled=true}]\n",
            true,
        ),
        ("", true),
        ("mcp_servers = [42]", false),
        (
            "mcp_servers = [{name='y',transport='stdio',command='echo',disabled='true'}]",
            false,
        ),
        (
            "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true},{name='_y_',transport='http'}]",
            false,
        ),
        (
            "mcp_servers = [{name='---',transport='stdio',command='echo',disabled=true}]",
            false,
        ),
    ] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            crate::command_pause::admit_internal_refresh(&core.shared, None),
            expected,
            "{text}"
        );
    }
    std::fs::remove_file(path).unwrap();
    assert!(crate::command_pause::admit_internal_refresh(
        &core.shared,
        None
    ));
    core.shutdown().await;
}
