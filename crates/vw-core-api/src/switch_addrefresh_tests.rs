//! Regression tests for deferred MCP changes with a person-disabled command server.
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
async fn add_and_change_defer_without_forced_discovery_in_two_chats() {
    let core = start_with_keys("addrefresh-two", None, true, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "fresh", 1, false, false, &calls, ok);
    chat(&core, "ran", 2, false, true, &calls, ok);
    for update in [false, true] {
        save(&core, update).await;
        assert_eq!(
            count(&calls, None, "mcp_catalog/refresh"),
            0,
            "save forced discovery"
        );
        for sid in ["fresh", "ran"] {
            assert!(core.shared.channels.switch_stale(sid));
            crate::switch_apply::before_work(
                &core.shared,
                "turn/start",
                &json!({"sessionId": sid}),
            )
            .await
            .unwrap();
            assert!(!core.shared.channels.switch_stale(sid));
        }
    }
    for sid in ["fresh", "ran"] {
        assert_eq!(count(&calls, Some(sid), "config/reload"), 2);
        assert_eq!(count(&calls, Some(sid), "mcp_catalog/toggle"), 2);
    }
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, method, _)| method == "mcp_catalog/toggle")
            .all(|(_, _, params)| params["name"] == "x" && params["disabled"] == false)
    );
    core.shutdown().await;
}

#[tokio::test]
async fn shared_refresh_defers_http_and_preserves_a_running_turn() {
    let config = "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true},{name='http',transport='http',url='http://127.0.0.1:9/mcp'}]\n";
    let core = start_with_keys("addrefresh-shared", None, true, config).await;
    let calls: Calls = Arc::default();
    chat(&core, "busy", 1, true, true, &calls, ok);
    chat(&core, "idle", 2, false, false, &calls, ok);
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    assert_eq!(count(&calls, None, "config/reload"), 0);
    assert!(core.shared.channels.switch_stale("busy"));
    assert!(core.shared.channels.has_channel("busy"));
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "idle"}))
        .await
        .unwrap();
    assert_eq!(count(&calls, Some("idle"), "mcp_catalog/toggle"), 1);
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, method, _)| method == "mcp_catalog/toggle")
            .all(|(_, _, params)| params["name"] == "http")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn forced_admission_and_both_raw_aliases_refuse_disabled_rows_while_on() {
    let core = start_with_keys("addrefresh-raw", None, true, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "idle", 1, false, false, &calls, ok);
    assert!(!crate::command_pause::admit_internal_refresh(
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
            assert!(result.is_err(), "{method} {origin:?} reached disabled Y");
        }
    }
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    assert_eq!(count(&calls, None, "mcp/refresh"), 0);
    core.shutdown().await;
}

#[tokio::test]
async fn forced_admission_checks_both_forms_and_empty_missing_malformed_inputs() {
    let core = start_with_keys("addrefresh-inputs", None, true, "").await;
    let path = core.shared.instructions.own_home.join("config.toml");
    for (text, expected) in [
        (DISABLED, false),
        (
            "[[mcp_servers]]\nname='y'\ntransport='stdio'\ncommand='echo'\ndisabled=true\n",
            false,
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

#[tokio::test]
async fn deferred_targets_are_rechecked_when_the_person_disables_x_before_apply() {
    let core = start_with_keys("addrefresh-later-off", None, true, DISABLED).await;
    let calls: Calls = Arc::default();
    chat(&core, "idle", 1, false, false, &calls, ok);
    save(&core, false).await;
    assert!(core.shared.channels.switch_stale("idle"));
    let path = core.shared.instructions.own_home.join("config.toml");
    std::fs::write(&path, "mcp_servers = [{name='y',transport='stdio',command='echo',disabled=true},{name='x',transport='stdio',command='echo',disabled=true}]").unwrap();
    crate::switch_apply::before_work(&core.shared, "turn/start", &json!({"sessionId": "idle"}))
        .await
        .unwrap();
    assert_eq!(count(&calls, None, "mcp_catalog/toggle"), 0);
    assert_eq!(count(&calls, None, "mcp_catalog/refresh"), 0);
    core.shutdown().await;
}
