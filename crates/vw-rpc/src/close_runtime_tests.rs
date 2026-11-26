//! Live close contract against an isolated Vibe 3.1.4 or 3.1.5 multiplex server.
//! Uses the loopback model fixture selected by `VW_CLOSE_RUNTIME_BRIDGE`.
//! Ignored by default: run with the three fixture environment variables below.

#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::{Connection, ConnectionConfig, MultiplexerBuilder};

struct Bridge {
    child: Child,
    pid: u32,
    reaped: bool,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if !self.reaped {
            // The wrapper and Vibe server share this test-owned process group.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", self.pid)])
                .status();
        }
    }
}

impl Bridge {
    async fn finish(mut self) {
        let status = tokio::time::timeout(Duration::from_secs(12), self.child.wait())
            .await
            .expect("isolated server exits after process stdin closes")
            .expect("wait for isolated server");
        self.reaped = true;
        assert!(status.success(), "bridge/server exited {status}");
    }
}

fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap()
        .to_owned()
}

async fn ready(conn: &Connection) {
    let init = conn
        .request(
            "initialize",
            json!({"clientInfo":{"name":"vw-close-contract-test","version":"1"},
                   "capabilities":{"callbackKinds":["approval"]}}),
        )
        .await
        .expect("initialize");
    assert!(init["serverInfo"]["version"].is_string());
    conn.notify("initialized", json!({}))
        .await
        .expect("initialized");
}

async fn start(conn: &Connection, cwd: &Path, mode: &str) -> String {
    let answer = conn
        .request(
            "session/start",
            json!({"agentConfig":{"cwd":cwd,"approvalPolicy":mode}}),
        )
        .await
        .expect("session/start");
    text(&answer, "/state/session/id")
}

async fn turn(conn: &Connection, session: &str, prompt: &str) {
    let answer = conn
        .request(
            "turn/start",
            json!({"sessionId":session,"message":[{"type":"text","text":prompt}]}),
        )
        .await
        .expect("turn/start");
    assert_eq!(answer["turn"]["status"], "in_progress");
}

async fn started(work: &Path, conn: &Connection, callbacks: &mut mpsc::UnboundedReceiver<Value>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if work.join("started.marker").exists() {
            return;
        }
        tokio::select! {
            Some(callback) = callbacks.recv() => {
                let id = text(&callback, "/callbackId");
                assert_eq!(callback["detail"]["kind"], "approval");
                let session = text(&callback, "/sessionId");
                let answer = conn.request("callback/result", json!({
                    "sessionId":session,
                    "result":{"callbackId":id,"output":{"type":"approval","decision":{"type":"approve"}}}
                })).await.expect("approve disposable shell tool");
                assert_eq!(answer["accepted"], true);
            }
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    panic!("scripted shell tool did not start; check bridge stderr");
}

async fn idle(conn: &Connection, session: &str) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        let read = conn
            .request("session/read", json!({"sessionId":session}))
            .await
            .expect("session/read");
        if read["state"]["session"]["status"]["type"] == "idle" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("second chat did not settle");
}

async fn one(server: &Path, bridge: &Path, root: &Path, mode: &str, process_close: bool) {
    assert!(
        !root.join("work/started.marker").exists() && !root.join("work/done.marker").exists(),
        "choose a fresh evidence root; prior tool markers must be retained"
    );
    std::fs::create_dir_all(root).unwrap();
    let stderr_path = root.join("bridge-stderr.log");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let mut command = Command::new("python3");
    command
        .arg(bridge)
        .arg(server)
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().expect("spawn isolated bridge");
    let pid = child.id().expect("bridge pid");
    let output = child.stdout.take().unwrap();
    let input = child.stdin.take().unwrap();
    let bridge_child = Bridge {
        child,
        pid,
        reaped: false,
    };
    let (callbacks_tx, mut callbacks_rx) = mpsc::unbounded_channel();
    let mux = MultiplexerBuilder::new(ConnectionConfig {
        interrupt_before_close: true,
        close_timeout: Duration::from_secs(5),
        ..ConnectionConfig::default()
    })
    .handler("callback/call", move |request: crate::ServerRequest| {
        let sender = callbacks_tx.clone();
        async move {
            let callback = request.params["callback"].clone();
            let id = callback["callbackId"].clone();
            sender.send(callback).unwrap();
            Ok(json!({"callbackId":id,"accepted":true}))
        }
    })
    .start(output, input);
    let first = mux.open_channel().expect("first channel");
    ready(&first).await;
    let work = root.join("work");
    let first_id = start(&first, &work, mode).await;
    turn(&first, &first_id, "Run the scripted shell tool").await;
    started(&work, &first, &mut callbacks_rx).await;
    assert!(
        !work.join("done.marker").exists(),
        "tool finished before close"
    );
    let second = mux.open_channel().expect("second channel");
    ready(&second).await;
    let second_id = start(&second, &work, "auto-approve").await;
    turn(&second, &second_id, "Other chat").await;
    if process_close {
        mux.close_input().await;
        tokio::time::timeout(Duration::from_secs(12), mux.closed())
            .await
            .expect("process close ack");
    } else {
        first.close_input().await;
        tokio::time::timeout(Duration::from_secs(12), first.closed())
            .await
            .expect("first close ack");
        idle(&second, &second_id).await;
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !work.join("done.marker").exists(),
        "shell tool continued after close"
    );
    if !process_close {
        mux.close_input().await;
        tokio::time::timeout(Duration::from_secs(12), mux.closed())
            .await
            .expect("process shutdown");
    }
    bridge_child.finish().await;
    println!(
        "LIVE_CLOSE_PASS mode={mode} kind={} server={} root={}",
        if process_close { "process" } else { "channel" },
        server.display(),
        root.display()
    );
}

/// Run with `VW_CLOSE_RUNTIME_BRIDGE`, `VW_CLOSE_SERVER` and `VW_CLOSE_TEST_ROOT` set to
/// the fixture script, a real Vibe server binary and an owned disposable evidence root.
#[tokio::test]
#[ignore = "requires explicit isolated Vibe server and disposable evidence root"]
async fn real_vibe_shell_tool_is_stopped_before_close() {
    let bridge = PathBuf::from(
        std::env::var_os("VW_CLOSE_RUNTIME_BRIDGE").expect("VW_CLOSE_RUNTIME_BRIDGE"),
    );
    let server = PathBuf::from(std::env::var_os("VW_CLOSE_SERVER").expect("VW_CLOSE_SERVER"));
    let root = PathBuf::from(std::env::var_os("VW_CLOSE_TEST_ROOT").expect("VW_CLOSE_TEST_ROOT"));
    for mode in ["ask", "accept-edits", "auto-approve"] {
        for process_close in [false, true] {
            let name = format!(
                "{mode}-{}",
                if process_close { "process" } else { "channel" }
            );
            one(&server, &bridge, &root.join(name), mode, process_close).await;
        }
    }
}
