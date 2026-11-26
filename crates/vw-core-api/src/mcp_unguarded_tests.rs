//! Real pinned Vibe direct-launch regression: disabled stdio must never start on shared refresh.
//! Uses an isolated home, two chats and a PID-writing MCP fixture; no model request is sent.

use std::sync::Arc;

use serde_json::json;

use crate::{Command, CommandResult, Core, CoreConfig, Origin};

const MCP_FIXTURE: &str = r#"import json, os, sys
with open(os.environ["FIXTURE_STARTS"], "a") as marker:
    marker.write(str(os.getpid()) + "\n")
for line in sys.stdin:
    message = json.loads(line)
    if "id" not in message:
        continue
    if message.get("method") == "initialize":
        result = {"protocolVersion": message["params"]["protocolVersion"],
                  "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}
    elif message.get("method") == "tools/list":
        result = {"tools": [{"name": "ping", "inputSchema": {"type": "object"}}]}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
"#;

struct PlaceholderKey;

impl vw_supervisor::SecretProvider for PlaceholderKey {
    fn mistral_api_key(&self) -> Result<Option<vw_supervisor::ApiKey>, vw_supervisor::SecretError> {
        Ok(Some(vw_supervisor::ApiKey::new(
            "vw-fixture-placeholder-not-a-key",
        )))
    }
}

#[tokio::test]
async fn no_sibling_python_shared_refresh_never_starts_disabled_stdio() {
    refresh_disabled(false).await;
}

#[tokio::test]
async fn sibling_python_guard_keeps_forced_refresh_working() {
    refresh_disabled(true).await;
}

fn fixture(
    root: &std::path::Path,
    guarded: bool,
) -> (
    vw_supervisor::SupervisorConfig,
    CoreConfig,
    std::path::PathBuf,
) {
    use std::os::unix::fs::PermissionsExt;

    let python = std::env::var_os("VIBE_PYTHON")
        .or_else(|| std::env::var_os("VW_VIBE_PYTHON"))
        .map_or_else(
            || {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../data")
                    .join(format!(
                        "vibe-{}/venv/bin/python",
                        include_str!("../../../tools/public/codegen/vibe.pin").trim()
                    ))
            },
            std::path::PathBuf::from,
        );
    let source = python
        .parent()
        .unwrap_or_else(|| panic!("Vibe Python has no parent directory"))
        .join("vibe-app-server");
    let bin = root.join("direct-bin");
    std::fs::create_dir(&bin).unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    let program = if guarded {
        source
    } else {
        // A copy named `vibe-app-server` is now either guarded through its venv shebang or refused before it starts.
        // The unguarded direct route that the shared-refresh rule must still handle is an unnamed program.
        let direct = bin.join("vibe-server-direct");
        std::fs::copy(source, &direct)
            .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
        std::fs::set_permissions(&direct, std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
        assert!(!bin.join("python").exists());
        direct
    };
    assert_eq!(vw_supervisor::adapter_python(&program).is_some(), guarded);
    let home = root.join("vibe-home");
    let data = root.join("app-data");
    std::fs::create_dir(&home)
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    std::fs::create_dir(&data)
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    std::fs::write(
        data.join("settings.json"),
        r#"{"developerPermissions":true}"#,
    )
    .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    let fixture = root.join("mcp.py");
    let marker = root.join("disabled-pids");
    std::fs::write(&fixture, MCP_FIXTURE)
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    let template = "active_model = 'large-4'\n[[models]]\nname = 'mistral-large-4'\nprovider = 'mistral'\nalias = 'large-4'\nmax_context_length = 1000000\n";
    let command = toml::Value::String(python.to_string_lossy().into_owned());
    let argument = toml::Value::String(fixture.to_string_lossy().into_owned());
    let starts = toml::Value::String(marker.to_string_lossy().into_owned());
    std::fs::write(home.join("config.toml"), format!(
        "enable_connectors = false\nenable_telemetry = false\n{template}\n[[mcp_servers]]\nname = 'person-disabled'\ntransport = 'stdio'\ncommand = {command}\nargs = [{argument}]\ndisabled = true\n[mcp_servers.env]\nFIXTURE_STARTS = {starts}\n"
    )).unwrap_or_else(|error| panic!("write Vibe config failed: {error:?}"));
    let mut server = vw_supervisor::SupervisorConfig::new(program, home, root.to_path_buf());
    std::fs::create_dir(root.join("home"))
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    server.home = Some(root.join("home"));
    server.extra_env.push((
        "PYTHON_KEYRING_BACKEND".into(),
        "keyring.backends.null.Keyring".into(),
    ));
    let mut config = CoreConfig {
        secret_scan_data: Some(data),
        instructions: crate::InstructionsConfig {
            cli_home: Some(root.join("cli")),
        },
        ..CoreConfig::default()
    };
    config.chrome_control.user_home = Some(root.join("browser-home"));
    (server, config, marker)
}

async fn refresh_disabled(guarded: bool) {
    let folder =
        tempfile::tempdir().unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    let root = folder.path();
    let (server, config, marker) = fixture(root, guarded);
    let core = Box::pin(Core::start(
        server,
        Arc::new(PlaceholderKey),
        crate::refresh_notice_tests::AlwaysAllow,
        config,
    ))
    .await
    .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    assert_eq!(
        core.shared.supervisor.mcp_refresh_guard_installed(),
        guarded
    );
    core.register_picked_folder(root)
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let result = core
            .execute(
                Origin::Ui,
                Command::StartSession {
                    agent_config: Some(
                        json!({"cwd":root,"approvalPolicy":"ask","trustWorkspace":true}),
                    ),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
        let CommandResult::Session { snapshot, .. } = result else {
            panic!("session must start")
        };
        sessions.push(snapshot.session_id);
    }
    assert!(
        !marker.exists(),
        "disabled fixture started before the shared refresh"
    );
    let mut events = core.subscribe(None);
    crate::connectors::refresh_servers(&core.shared).await;
    let starts = std::fs::read_to_string(&marker).unwrap_or_default();
    if !starts.is_empty() {
        core.shutdown().await;
        panic!("unguarded shared forced refresh started disabled stdio: {starts}");
    }
    if !guarded {
        let notices = crate::refresh_notice_tests::tools_alerts(
            &crate::refresh_notice_tests::drain_events(&mut events).await,
        );
        assert!(
            notices.iter().any(|(_, message)| message
                == vw_vocabulary::texts::COMMAND_SERVER_DISABLED_REFRESH_BLOCKED)
        );
    }
    exercise_refreshes(&core, &sessions, guarded, &marker).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !marker.exists(),
        "disabled stdio started after refresh returned"
    );
    core.shutdown().await;
    assert!(!core.shared.supervisor.mcp_refresh_guard_installed());
    println!(
        "FIX19_UNGUARDED guarded={guarded} sibling_python={guarded} chats={} modes=3 switch_states=2 disabled_starts=0",
        sessions.len()
    );
}

async fn exercise_refreshes(
    core: &Core,
    sessions: &[String],
    guarded: bool,
    marker: &std::path::Path,
) {
    for enabled in [true, false] {
        core.execute(
            Origin::Ui,
            Command::DeveloperPermissions {
                enabled: Some(enabled),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
        for policy in ["ask", "accept-edits", "auto-approve"] {
            for session_id in sessions {
                core.execute(
                    Origin::Ui,
                    Command::SetAccessMode {
                        session_id: session_id.clone(),
                        agent_name: policy.into(),
                        confirmation_token: None,
                    },
                )
                .await
                .unwrap_or_else(|error| panic!("fixture operation failed: {error:?}"));
            }
            crate::connectors::refresh_servers(&core.shared).await;
            for method in ["mcp_catalog/refresh", "mcp/refresh"] {
                for origin in [Origin::Ui, Origin::DebugApi] {
                    let call = |session_id: &str| Command::Call {
                        method: method.into(),
                        params: json!({"sessionId":session_id}),
                        confirmation_token: None,
                    };
                    let (first, second) = tokio::join!(
                        core.execute(origin, call(&sessions[0])),
                        core.execute(origin, call(&sessions[1])),
                    );
                    for result in [first, second] {
                        if guarded {
                            assert!(
                                result.is_ok(),
                                "{policy} {enabled} {method} {origin:?}: {result:?}"
                            );
                        } else {
                            assert!(
                                matches!(result, Err(error) if error.code == crate::ErrorCode::Denied)
                            );
                        }
                    }
                }
            }
            assert!(
                !marker.exists(),
                "{policy} enabled={enabled}: disabled stdio started"
            );
        }
    }
}
