//! Core tests: withheld internal MCP refreshes emit a single deduplicated alert
//! per Developer-permissions switch-off period and cause, without refreshing or starting children,
//! and agent-requested forwarded refreshes carry user-directed reason and recovery wording.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use crate::{
    AlertKind, Command, ConnectorsRequest, Core, CoreEvent, CoreSubscription, ErrorCode, Origin,
    SecretsRequest,
};

pub(crate) struct AlwaysAllow;

impl Confirmer for AlwaysAllow {
    fn confirm(
        &self,
        _: &ConfirmationRequest,
    ) -> impl std::future::Future<Output = Confirmation> + Send {
        std::future::ready(Confirmation::ApprovedAlways)
    }
}

pub(crate) async fn memory_keys(root: &Path) -> Arc<vw_keys::KeyService> {
    let keys = vw_keys::KeyService::start(vw_keys::KeyServiceConfig {
        managed_vault: None,
        vault: vw_vault::VaultConfig {
            location: vw_vault::location::VaultLocation::explicit(root.join("vault")),
            device_store: Arc::new(vw_vault::remember::MemoryDeviceStore::default()),
            kdf: vw_vault::KdfParams::default(),
            device_name: "test".into(),
        },
        proxy: vw_keyproxy::ProxyConfig::default(),
        test_api_base: "http://127.0.0.1:1".into(),
        settings_path: root.join("keys.json"),
        existing: vw_keys::ExistingKeys {
            dotenv: root.join(".env"),
            keyring: Arc::new(vw_keys::existing::MemoryVibeKeyring::default()),
        },
    })
    .await
    .unwrap();
    keys.create_vault(vw_vault::SecretString::new("refresh notice test vault"))
        .await
        .unwrap();
    keys
}

pub(crate) async fn start_with_keys(
    name: &str,
    keys: Option<Arc<vw_keys::KeyService>>,
    developer_permissions: bool,
    config_toml: &str,
) -> Core {
    let config_body = config_toml.to_owned();
    crate::offline_core::start_with_confirmer(
        name,
        move |config, dir| {
            let app_data = dir.join("app-data");
            let vibe_home = dir.join("vibe-home");
            std::fs::create_dir_all(&app_data).unwrap();
            std::fs::create_dir_all(&vibe_home).unwrap();
            std::fs::write(
                app_data.join("settings.json"),
                json!({"developerPermissions": developer_permissions}).to_string(),
            )
            .unwrap();
            if !config_body.is_empty() {
                std::fs::write(vibe_home.join("config.toml"), &config_body).unwrap();
            }
            config.secret_scan_data = Some(app_data);
            config.connection_wait = Duration::from_millis(20);
            config.keys = keys;
        },
        AlwaysAllow,
    )
    .await
}

pub(crate) fn attach_mock_channel(
    core: &Core,
    session_id: &str,
    channel_id: u64,
    recorded_methods: Arc<Mutex<Vec<(String, String)>>>,
) -> tokio::task::JoinHandle<()> {
    let _ = core.shared.store.load(
        json!({
            "session": {
                "id": session_id,
                "cwd": core.shared.default_cwd,
                "status": {"type": "idle"}
            },
            "entries": [],
            "turns": []
        }),
        vw_session::LoadKind::New,
    );
    let (client, server) = tokio::io::duplex(16384);
    let (read, write) = tokio::io::split(client);
    let conn =
        vw_rpc::ConnectionBuilder::new(vw_rpc::ConnectionConfig::default()).start(read, write);
    core.shared
        .channels
        .bind_test_peer(session_id, conn, (0, channel_id));
    let sid = session_id.to_owned();
    tokio::spawn(async move {
        let (read, mut write) = tokio::io::split(server);
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let msg: Value = serde_json::from_str(&line).unwrap();
            let method = msg["method"].as_str().unwrap_or_default().to_owned();
            recorded_methods.lock().unwrap().push((sid.clone(), method));
            if let Some(id) = msg.get("id") {
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": {}});
                write
                    .write_all(format!("{reply}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    })
}

pub(crate) async fn drain_events(sub: &mut CoreSubscription) -> Vec<CoreEvent> {
    let mut events = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), sub.next()).await {
        events.push(event);
    }
    events
}

pub(crate) fn tools_alerts(events: &[CoreEvent]) -> Vec<(Option<String>, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            CoreEvent::Alert {
                session_id,
                kind: AlertKind::ToolsUnavailable,
                message,
            } => Some((session_id.clone(), message.clone())),
            _ => None,
        })
        .collect()
}

pub(crate) fn count_server_refreshes(calls: &Mutex<Vec<(String, String)>>) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, method)| method == "mcp_catalog/refresh")
        .count()
}

pub(crate) fn assert_paused_notice_text(message: &str) {
    let name = vw_vocabulary::texts::DEVELOPER_PERMISSIONS_NAME;
    assert!(
        message.contains(&format!(
            "an MCP command server is paused while {name} is off"
        )),
        "missing paused reason: {message}"
    );
    assert!(
        message.contains(name),
        "missing switch name in the notice: {message}"
    );
    assert!(
        message.contains("remove the paused command server in Tools")
            && message.contains("refresh"),
        "missing Tools removal + refresh action: {message}"
    );
}

async fn assert_no_tools_alerts(sub: &mut CoreSubscription, context: &str) {
    let alerts = tools_alerts(&drain_events(sub).await);
    assert_eq!(alerts, Vec::<(Option<String>, String)>::new(), "{context}");
}

async fn set_developer_permissions(core: &Core, enabled: bool) {
    core.execute(
        Origin::Ui,
        Command::DeveloperPermissions {
            enabled: Some(enabled),
        },
    )
    .await
    .unwrap();
}

async fn assert_inspect_failure_deduped(
    core: &Core,
    config_path: &Path,
    calls: &Mutex<Vec<(String, String)>>,
    sub: &mut CoreSubscription,
) {
    std::fs::write(config_path, "mcp_servers = [42]\n").unwrap();
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(count_server_refreshes(calls), 0);
    let inspect_alerts = tools_alerts(&drain_events(sub).await);
    assert_eq!(
        inspect_alerts.len(),
        1,
        "distinct inspect-failure cause should emit once: {inspect_alerts:?}"
    );
    assert_eq!(
        inspect_alerts[0].1,
        vw_vocabulary::texts::COMMAND_SERVER_INTERNAL_REFRESH_UNCHECKABLE,
        "unexpected inspect-failure alert: {:?}",
        inspect_alerts[0]
    );
    assert!(inspect_alerts[0].1.contains("config.toml"));
    crate::connectors::refresh_servers(&core.shared).await;
    assert_no_tools_alerts(sub, "repeated inspect-failure trigger must be deduplicated").await;
}

#[tokio::test]
async fn grant_revoke_and_switch_reset_deduplicate_withheld_internal_refresh_alerts() {
    let key_root = std::env::temp_dir().join(format!(
        "vw-refresh-notice-keys-{}-{}",
        std::process::id(),
        vw_changes::now_ms()
    ));
    let keys = memory_keys(&key_root).await;
    let core = start_with_keys("refresh-note-grant-revoke", Some(keys), false, "").await;

    let imported = crate::mcp_keys::import_key_server(
        &core.shared,
        "docs",
        "https://docs.example.com/mcp",
        "Authorization",
        "Bearer docs-secret-123".to_owned(),
    )
    .await
    .unwrap();
    let config_path = core.shared.instructions.own_home.join("config.toml");
    let mut toml = std::fs::read_to_string(&config_path).unwrap();
    toml.push_str(
        "\n[[mcp_servers]]\nname = \"local-stdio\"\ntransport = \"stdio\"\ncommand = \"/nonexistent/stdio-server\"\ndisabled = true\n",
    );
    std::fs::write(&config_path, &toml).unwrap();

    let calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let _p1 = attach_mock_channel(&core, "s1", 1, calls.clone());
    let _p2 = attach_mock_channel(&core, "s2", 2, calls.clone());

    let mut sub = core.subscribe(None);
    let _ = drain_events(&mut sub).await;

    // 1. Granting a key while OFF with a paused stdio server withholds refresh and emits ONE visible alert.
    core.execute(
        Origin::Ui,
        Command::Connectors {
            request: ConnectorsRequest::AllowServerKey {
                name: "docs".to_owned(),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        count_server_refreshes(&calls),
        0,
        "refresh must stay withheld"
    );
    let alerts = tools_alerts(&drain_events(&mut sub).await);
    assert_eq!(alerts.len(), 1, "expected one withheld alert: {alerts:?}");
    assert_eq!(alerts[0].0, None);
    assert_paused_notice_text(&alerts[0].1);

    // 2. Revoking the grant while still in the same switch-off period emits no second alert and no refresh.
    core.execute(
        Origin::Ui,
        Command::Secrets {
            request: SecretsRequest::Revoke {
                grant_id: imported.binding.clone(),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        count_server_refreshes(&calls),
        0,
        "refresh must stay withheld"
    );
    assert_no_tools_alerts(
        &mut sub,
        "second trigger in same OFF period must be deduplicated",
    )
    .await;

    // 3. A distinct cause (uncheckable config.toml) in the same switch-off period emits once and dedupes on repeat.
    assert_inspect_failure_deduped(&core, &config_path, &calls, &mut sub).await;

    // 4. ON with a person-disabled row defers each chat; a config with every command enabled can force refresh.
    std::fs::write(&config_path, &toml).unwrap();
    set_developer_permissions(&core, true).await;
    let _ = drain_events(&mut sub).await;
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(count_server_refreshes(&calls), 0);
    assert!(core.shared.channels.switch_stale("s1"));
    assert!(core.shared.channels.switch_stale("s2"));
    assert_no_tools_alerts(&mut sub, "deferred convergence is not a withheld refresh").await;
    std::fs::write(
        &config_path,
        toml.replace("disabled = true", "disabled = false"),
    )
    .unwrap();
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(
        count_server_refreshes(&calls),
        2,
        "both open chats must refresh when ON"
    );
    assert_no_tools_alerts(&mut sub, "ON refresh must not emit withheld alert").await;

    set_developer_permissions(&core, false).await;
    let _ = drain_events(&mut sub).await;
    crate::connectors::refresh_servers(&core.shared).await;
    assert_eq!(count_server_refreshes(&calls), 2);
    let new_period_alerts = tools_alerts(&drain_events(&mut sub).await);
    assert_eq!(
        new_period_alerts.len(),
        1,
        "new OFF period should emit one alert: {new_period_alerts:?}"
    );
    assert_paused_notice_text(&new_period_alerts[0].1);

    core.shutdown().await;
    let _ = std::fs::remove_dir_all(&key_root);
}

#[tokio::test]
async fn forwarded_refresh_refusal_uses_person_or_user_wording_by_origin() {
    let core = start_with_keys(
        "refresh-note-forwarded",
        None,
        false,
        "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = true }]\n",
    )
    .await;
    let mut sub = core.subscribe(None);
    let _ = drain_events(&mut sub).await;

    for method in ["mcp_catalog/refresh", "mcp/refresh"] {
        let ui_err = core
            .execute(
                Origin::Ui,
                Command::Call {
                    method: method.to_owned(),
                    params: json!({}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(ui_err.code, ErrorCode::Denied);
        assert!(
            ui_err.message.contains("MCP refresh was refused"),
            "{ui_err}"
        );
        assert_paused_notice_text(&ui_err.message);
        assert!(
            !ui_err.message.to_ascii_lowercase().contains("the user"),
            "UI refusal should address the person directly: {ui_err}"
        );

        let agent_err = core
            .execute(
                Origin::DebugApi,
                Command::Call {
                    method: method.to_owned(),
                    params: json!({}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(agent_err.code, ErrorCode::Denied);
        assert!(
            agent_err.message.contains("MCP refresh was refused"),
            "{agent_err}"
        );
        assert_paused_notice_text(&agent_err.message);
        assert!(
            agent_err.message.to_ascii_lowercase().contains("the user"),
            "agent refusal must refer to the user: {agent_err}"
        );
    }

    let events = drain_events(&mut sub).await;
    assert_eq!(
        tools_alerts(&events),
        Vec::<(Option<String>, String)>::new(),
        "forwarded refusals must not emit duplicate internal alerts"
    );
    let refused_reasons: Vec<String> = events
        .into_iter()
        .filter_map(|event| match event {
            CoreEvent::Refused {
                origin: Origin::DebugApi,
                reason,
                ..
            } => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(refused_reasons.len(), 2);
    assert!(refused_reasons[0].to_ascii_lowercase().contains("the user"));

    core.shutdown().await;
}

#[tokio::test]
async fn empty_config_refreshes_while_off_and_paused_stdio_withholds_across_access_modes() {
    // 1. Empty config while OFF: internal refresh proceeds with zero alerts across open chats.
    let empty_core = start_with_keys("refresh-note-empty-config", None, false, "").await;
    let empty_calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let _e1 = attach_mock_channel(&empty_core, "e1", 1, empty_calls.clone());
    let _e2 = attach_mock_channel(&empty_core, "e2", 2, empty_calls.clone());
    let mut empty_sub = empty_core.subscribe(None);
    let _ = drain_events(&mut empty_sub).await;

    crate::connectors::refresh_servers(&empty_core.shared).await;
    assert_eq!(
        count_server_refreshes(&empty_calls),
        2,
        "empty config should allow normal refresh while OFF"
    );
    assert_no_tools_alerts(&mut empty_sub, "empty config must emit no alert").await;
    empty_core.shutdown().await;

    // 2. Paused stdio while OFF with open chats in Ask, Accept edits, and Full access modes:
    //    withheld across all modes with a single deduplicated alert.
    let paused_core = start_with_keys(
        "refresh-note-access-modes",
        None,
        false,
        "mcp_servers = [{ name = \"local\", transport = \"stdio\", command = \"/bin/echo\", disabled = true }]\n",
    )
    .await;
    let paused_calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    for (idx, (sid, mode)) in [
        ("ask-chat", "ask"),
        ("edit-chat", "accept-edits"),
        ("full-chat", "auto-approve"),
    ]
    .into_iter()
    .enumerate()
    {
        let _peer = attach_mock_channel(
            &paused_core,
            sid,
            u64::try_from(idx + 1).unwrap(),
            paused_calls.clone(),
        );
        paused_core.shared.access.observe_request(
            "session/approvalPolicy/update",
            sid,
            &json!({"approvalPolicy": mode}),
        );
    }
    let mut sub = paused_core.subscribe(None);
    let _ = drain_events(&mut sub).await;

    crate::connectors::refresh_servers(&paused_core.shared).await;
    assert_eq!(count_server_refreshes(&paused_calls), 0);
    let alerts = tools_alerts(&drain_events(&mut sub).await);
    assert_eq!(alerts.len(), 1);
    assert_paused_notice_text(&alerts[0].1);

    crate::connectors::refresh_servers(&paused_core.shared).await;
    assert_eq!(count_server_refreshes(&paused_calls), 0);
    assert_no_tools_alerts(&mut sub, "second trigger across modes must be deduplicated").await;
    paused_core.shutdown().await;
}
