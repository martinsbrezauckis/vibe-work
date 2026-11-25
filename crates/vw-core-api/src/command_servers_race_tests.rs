//! Deterministic save versus Developer-switch interleaving for command MCP servers.
//! Uses the test-only seam immediately before the config edit; the core still handles both real commands.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

use crate::command::{Command, Origin};
use crate::command_servers::save_test_seam::{self, Hold};
use crate::{Core, CoreConfig};

struct Approve;

impl Confirmer for Approve {
    fn confirm(&self, _: &ConfirmationRequest) -> impl std::future::Future<Output = Confirmation> {
        std::future::ready(Confirmation::Approved)
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The test keeps the forced save, OFF request and two inspections in one visible order.
async fn off_during_an_approved_save_leaves_the_command_server_paused() {
    let root = std::env::temp_dir().join(format!(
        "fix18-cmdgate-race-{}-{}",
        std::process::id(),
        vw_changes::now_ms()
    ));
    let app_data = root.join("app-data");
    let vibe_home = root.join("vibe-home");
    std::fs::create_dir_all(&app_data).unwrap();
    std::fs::create_dir_all(&vibe_home).unwrap();
    std::fs::write(
        app_data.join("settings.json"),
        r#"{"developerPermissions":true}"#,
    )
    .unwrap();
    let config = CoreConfig {
        start_vibe: false,
        secret_scan_data: Some(app_data),
        ..CoreConfig::default()
    };
    let core = Arc::new(
        Box::pin(Core::start(
            vw_supervisor::SupervisorConfig::new(
                root.join("no-vibe"),
                vibe_home.clone(),
                root.clone(),
            ),
            Arc::new(vw_supervisor::NoSecret),
            Approve,
            config,
        ))
        .await
        .unwrap(),
    );
    let hold = Arc::new(Hold {
        name: "race-command-18".to_owned(),
        reached: tokio::sync::Semaphore::new(0),
        release: tokio::sync::Semaphore::new(0),
        wrote: tokio::sync::Semaphore::new(0),
        finish: tokio::sync::Semaphore::new(0),
    });
    save_test_seam::set(Some(hold.clone()));
    let add: Command = serde_json::from_value(json!({
        "type": "connectors",
        "request": {
            "op": "addCommandServer",
            "name": "race-command-18",
            "command": std::env::current_exe().unwrap().to_string_lossy(),
            "args": [], "env": []
        }
    }))
    .unwrap();
    let adding = {
        let core = core.clone();
        tokio::spawn(async move { core.execute(Origin::Ui, add).await })
    };
    tokio::time::timeout(Duration::from_secs(10), hold.reached.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let mut off = {
        let core = core.clone();
        tokio::spawn(async move {
            core.execute(
                Origin::Ui,
                Command::DeveloperPermissions {
                    enabled: Some(false),
                },
            )
            .await
        })
    };
    // The old implementation completes OFF while the save is held, then writes an enabled entry.
    // With the shared lock OFF waits; either way, release the save and inspect the final config.
    let off_finished = tokio::time::timeout(Duration::from_secs(1), &mut off).await;
    hold.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), hold.wrote.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let mid_doc: toml::Table = std::fs::read_to_string(vibe_home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let mid_server = mid_doc["mcp_servers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|row| row["name"].as_str() == Some("race-command-18"))
        .unwrap();
    let safe_before_refresh = core.shared.developer_permissions.enabled()
        || mid_server.get("disabled").and_then(toml::Value::as_bool) == Some(true);
    hold.finish.add_permits(1);
    adding.await.unwrap().unwrap();
    match off_finished {
        Ok(result) => result.unwrap().unwrap(),
        Err(_) => off.await.unwrap().unwrap(),
    };
    let doc: toml::Table = std::fs::read_to_string(vibe_home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let server = doc["mcp_servers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|row| row["name"].as_str() == Some("race-command-18"))
        .unwrap();
    assert_eq!(
        server.get("disabled").and_then(toml::Value::as_bool),
        Some(true)
    );
    assert!(
        safe_before_refresh,
        "an enabled command entry was written after the switch turned off, before refresh"
    );
    save_test_seam::set(None);
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}
