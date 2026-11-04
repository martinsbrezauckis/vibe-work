//! Independent receipts for Vibe's background processes, before their reader can reap the pid.
//! The pinned launcher authenticates over loopback; Core verifies launcher-observed native identities in vw-procmon.
//! Called by Core startup and the session panel. Turn cancellation and missing Vibe snapshots never
//! erase receipts. No project files or inferred command/cwd matches can register a process.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use vw_procmon::{OwnProcess, OwnProcessRegistry, ProcessClass, ProcessGroup, ProcessRef};

use crate::error::CoreStartError;

#[cfg(test)]
#[path = "vibe_processes_listener_tests.rs"]
mod listener_tests;

#[cfg(all(test, unix))]
#[path = "vibe_processes_registration_tests.rs"]
mod registration_tests;

#[derive(Clone)]
pub(crate) struct Tracked {
    pub(crate) session: String,
    pub(crate) id: String,
    pub(crate) command: String,
    pub(crate) process: ProcessRef,
}

type Records = Arc<Mutex<HashMap<(String, String), Tracked>>>;

/// Owned by Core, including its authenticated listener and monitor registrations.
pub(crate) struct VibeProcesses {
    records: Records,
    registry: OwnProcessRegistry,
    task: JoinHandle<()>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    session: String,
    id: String,
    command: String,
    pid: u32,
    start_token: u64,
}

impl VibeProcesses {
    pub(crate) async fn start(
        config: &mut vw_supervisor::SupervisorConfig,
        monitor: &vw_procmon::ProcMon,
    ) -> Result<Self, CoreStartError> {
        let fail = |e: String| CoreStartError::ProcessTracking(e);
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|e| fail(e.to_string()))?;
        let mut token = String::with_capacity(64);
        for byte in random {
            let _ = write!(token, "{byte:02x}");
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| fail(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| fail(e.to_string()))?
            .port();
        config.extra_env.push((
            "VW_PROCESS_TRACKER".into(),
            format!("{port}:{token}").into(),
        ));
        let records = Records::default();
        let registry = monitor.registry().clone();
        let held = records.clone();
        let registrations = registry.clone();
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    // Reap completed work even while the accept queue stays busy.
                    biased;
                    _ = clients.join_next(), if !clients.is_empty() => {}
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let Ok(socket) = authenticate(socket, &token) else { continue };
                        if clients.len() >= 16 {
                            continue;
                        }
                        let (records, registry) = (held.clone(), registrations.clone());
                        clients.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(5),
                                receive(socket, records, registry)).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            records,
            registry,
            task,
        })
    }

    pub(crate) fn shutdown(&self) {
        self.task.abort();
    }

    /// Live receipts, pruned only by OS identity. A missing upstream listing is not an exit.
    pub(crate) fn live(&self, session: &str) -> Vec<Tracked> {
        let snapshot: Vec<_> = self
            .records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        let mut live = Vec::new();
        for item in snapshot {
            if vw_procmon::is_alive(item.process) {
                if item.session == session {
                    live.push(item);
                }
            } else {
                self.registry.unregister_process(item.process);
                let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
                let key = (item.session, item.id);
                if records
                    .get(&key)
                    .is_some_and(|current| current.process == item.process)
                {
                    records.remove(&key);
                }
            }
        }
        live
    }
}

impl Drop for VibeProcesses {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// No task, timer, buffer growth or waiting slot exists before authentication.
/// The token and newline must already be queued; even a partial header is dropped.
/// The launcher sends header + receipt together and retries arrival/fragmentation races.
fn authenticate(socket: TcpStream, token: &str) -> std::io::Result<TcpStream> {
    // Tokio's accepted stream may not yet have cached readiness. Read the underlying
    // nonblocking socket once so already-queued bytes work on every platform.
    let mut socket = socket.into_std()?;
    let mut header = [0_u8; 65];
    let read = std::io::Read::read(&mut socket, &mut header)?;
    if read != header.len()
        || header[64] != b'\n'
        || !bool::from(header[..64].ct_eq(token.as_bytes()))
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    TcpStream::from_std(socket)
}

async fn receive(
    socket: TcpStream,
    records: Records,
    registry: OwnProcessRegistry,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.take(1024 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if !line.ends_with('\n') {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let receipt: Receipt = serde_json::from_str(&line)?;
    if receipt.session.is_empty() || receipt.id.is_empty() {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    }
    let tracked = register(receipt, records, registry, vw_procmon::identity).await?;
    reader
        .into_inner()
        .into_inner()
        .write_all(if tracked { b"ok\n" } else { b"gone\n" })
        .await
}

async fn register(
    receipt: Receipt,
    records: Records,
    registry: OwnProcessRegistry,
    lookup: impl FnOnce(u32) -> Option<ProcessRef> + Send + 'static,
) -> std::io::Result<bool> {
    // Blocking work only reads: dropping the receive future (timeout/shutdown) must
    // leave no detached task able to publish ownership after the launcher cleans up.
    let pid = receipt.pid;
    let process = tokio::task::spawn_blocking(move || lookup(pid))
        .await
        .map_err(std::io::Error::other)?;
    let expected = ProcessRef {
        pid,
        start: vw_procmon::StartToken(receipt.start_token),
    };
    if process != Some(expected) {
        return Ok(false);
    }
    // The receipt binds the identity the launcher observed before retries/cleanup.
    // Even if lookup was delayed past PID reuse, a replacement cannot become ours.
    let mut records = records.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (receipt.session.clone(), receipt.id.clone());
    if let Some(old) = records.get(&key) {
        if old.process == expected {
            return Ok(true);
        }
        registry.unregister_process(old.process);
    }
    let _ = registry.register(OwnProcess {
        session_id: Some(receipt.session.clone()),
        process: expected,
        group: ProcessGroup::None,
        class: ProcessClass::Agent,
        label: "background process".into(),
        keep_running: false,
    });
    records.insert(
        key,
        Tracked {
            session: receipt.session,
            id: receipt.id,
            command: receipt.command,
            process: expected,
        },
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeping_child() -> tokio::process::Child {
        #[cfg(unix)]
        let mut command = {
            // The Unix sleep utility owns its PID for the whole fixture lifetime.
            let mut command = tokio::process::Command::new("sleep");
            command.arg("30");
            command
        };
        #[cfg(windows)]
        let mut command = {
            // cmd's built-in read waits on Child's retained stdin pipe; no grandchild can outlive cleanup.
            let mut command = tokio::process::Command::new("cmd.exe");
            command
                .args(["/D", "/Q", "/C", "set", "/p", "receipt_wait="])
                .stdin(std::process::Stdio::piped())
                .creation_flags(0x0800_0000);
            command
        };
        command.kill_on_drop(true).spawn().unwrap()
    }

    #[allow(clippy::unwrap_used)]
    async fn send(config: &vw_supervisor::SupervisorConfig, receipt: serde_json::Value) -> String {
        let endpoint = config
            .extra_env
            .iter()
            .find(|(name, _)| name == "VW_PROCESS_TRACKER")
            .unwrap()
            .1
            .to_str()
            .unwrap();
        let (port, _) = endpoint.split_once(':').unwrap();
        listener_tests::report(format!("127.0.0.1:{port}"), receipt).await
    }

    #[tokio::test]
    async fn authenticated_receipts_keep_exact_identities_until_exit() {
        let monitor = vw_procmon::ProcMon::new(vw_procmon::ProcMonConfig::default()).unwrap();
        let mut config = vw_supervisor::SupervisorConfig::new(
            "/unused".into(),
            "/unused".into(),
            "/unused".into(),
        );
        let tracker = VibeProcesses::start(&mut config, &monitor).await.unwrap();
        let mut child = sleeping_child();
        let pid = child.id().unwrap();
        let identity = vw_procmon::identity(pid).unwrap();
        #[cfg(any(windows, target_os = "macos"))]
        assert_eq!(
            identity.start.0,
            vw_platform::process_facts(pid).unwrap().start_token,
            "receipt identity must retain the native start token"
        );
        let mut receipt = serde_json::json!({"token":"wrong", "session":"one", "id":"process-one",
            "command":"test process", "pid":pid, "startToken":identity.start.0});
        assert_eq!(send(&config, receipt.clone()).await, "");
        assert_eq!(monitor.registry().list(), []);
        receipt["token"] = config
            .extra_env
            .last()
            .unwrap()
            .1
            .to_str()
            .unwrap()
            .split_once(':')
            .unwrap()
            .1
            .into();
        assert_eq!(send(&config, receipt.clone()).await, "ok\n");
        assert_eq!(send(&config, receipt).await, "ok\n");
        assert_eq!(monitor.registry().list().len(), 1, "idempotent receipt");
        assert!(tracker.live("two").is_empty(), "session isolation");
        assert_eq!(tracker.live("one")[0].process, identity);
        child.kill().await.unwrap();
        child.wait().await.unwrap();
        assert!(tracker.live("one").is_empty());
        assert_eq!(monitor.registry().list(), []);
        // A stale receipt with a live, reused pid must not appear or signal that pid.
        let own = vw_procmon::identity(std::process::id()).unwrap();
        let mut stale = own;
        stale.start.0 += 1;
        tracker.records.lock().unwrap().insert(
            ("one".into(), "stale".into()),
            Tracked {
                session: "one".into(),
                id: "stale".into(),
                command: "old process".into(),
                process: stale,
            },
        );
        assert!(tracker.live("one").is_empty());
        assert!(vw_procmon::is_alive(own));
    }
}
