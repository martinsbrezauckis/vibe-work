//! Real child process close tests: an unacknowledged interrupt must kill the whole process tree.
//! Linux only: the fixture uses /usr/bin/python3 and Unix process-group containment.

#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;

use crate::{NoSecret, Supervisor, SupervisorConfig, SupervisorState};

static NEXT: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    marker: PathBuf,
    supervisor: Supervisor,
}

impl Fixture {
    async fn start(mode: &str) -> Self {
        let root = std::env::var_os("VW_TEST_ROOT")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(format!(
                "vw-supervisor-close-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        let program = root.join("close-test-peer");
        std::fs::write(&program, include_str!("close_test_peer.py")).unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&program, permissions).unwrap();
        let marker = root.join("tool");
        let mut config = SupervisorConfig::new(program, root.join("vibe-home"), root.join("work"));
        config.home = Some(root.join("home"));
        config.parent_env = Some(Vec::new());
        config.extra_args = vec![
            "--marker".into(),
            marker.as_os_str().to_owned(),
            "--mode".into(),
            mode.into(),
        ];
        config.connection.close_timeout = Duration::from_millis(250);
        config.shutdown_grace = Duration::from_millis(250);
        config.backoff.max_attempts = 0;
        let supervisor = Supervisor::start(config, Arc::new(NoSecret)).await.unwrap();
        Self { marker, supervisor }
    }

    async fn running_channel(&self) -> vw_rpc::Connection {
        let opened = self
            .supervisor
            .open_channel(Duration::from_secs(2))
            .await
            .unwrap();
        let reply = opened
            .conn
            .request("session/start", json!({}))
            .await
            .unwrap();
        assert_eq!(reply["state"]["session"]["id"], "chat-a");
        wait_file(&self.marker.with_extension("started")).await;
        opened.conn
    }
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture marker appeared");
}

async fn assert_tool_stopped(fixture: &Fixture) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        !fixture.marker.with_extension("done").exists(),
        "the tool child finished after its chat/server closed"
    );
}

#[tokio::test]
async fn missing_interrupt_ack_kills_the_server_and_its_tool_child() {
    let fixture = Fixture::start("ignore").await;
    let conn = fixture.running_channel().await;
    conn.close_input().await;
    let mut state = fixture.supervisor.state();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(&*state.borrow_and_update(), SupervisorState::Failed { .. }) {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("unacknowledged interrupt kills the server within the bound");
    assert_tool_stopped(&fixture).await;
}

#[tokio::test]
async fn supervisor_stop_interrupts_before_stdin_eof() {
    let fixture = Fixture::start("ack").await;
    let _conn = fixture.running_channel().await;
    let _ = fixture.supervisor.stop().await;
    assert!(
        fixture.marker.with_extension("interrupt").exists(),
        "supervisor stop must send and await the interrupt before stdin EOF"
    );
    assert_tool_stopped(&fixture).await;
}
