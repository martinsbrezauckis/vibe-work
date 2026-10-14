//! Fake Vibe and agent-tools integration helpers for disposable tests.
//! They start test binaries and call the local tool wire; callers own all processes and folders.
#![allow(dead_code, unreachable_pub)]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use vw_agent_tools::wire;
use vw_core_api::{AgentToolsConfig, Command, CommandResult, Core, CoreConfig, Origin};
use vw_procmon::LeaseConfig;

use crate::common::core::CanaryKey;
use crate::common::fake::{fake_config, scratch};
use crate::common::gate::FakeConfirmer;

/// The test build of the `vw-agent-tools` program.
pub const BIN: &str = env!("CARGO_BIN_EXE_vw_agent_tools_bin");

/// A lease range of 24 ports inside this test binary's own claimed block (`procmon::port`, the per-run port
/// blocks: two worktrees never share one), below Linux's ephemeral range. Each call gets the
/// next range of the block, so tests running in parallel in one binary never share ports either.
pub fn per_run_leases() -> LeaseConfig {
    static NEXT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    assert!(n < 20, "at most 20 lease ranges per test binary");
    let first = crate::procmon::port(500 + n * 24);
    LeaseConfig {
        first,
        last: first + 23,
        per_session: 4,
    }
}

/// Folders shared by the cores of one test (a restart reuses them).
#[derive(Clone)]
pub struct Dirs {
    pub root: PathBuf,
    pub ledger: PathBuf,
    pub data: PathBuf,
    pub persist: PathBuf,
    pub leases: LeaseConfig,
}

impl Dirs {
    pub fn new(name: &str) -> Self {
        let root = scratch(name);
        Self {
            ledger: root.join("ledger"),
            data: root.join("agent-tools"),
            persist: root.join("fake-sessions.json"),
            leases: per_run_leases(),
            root,
        }
    }
}

pub struct Rig {
    pub core: Core,
    pub dirs: Dirs,
    /// `{"method", "agentConfig"}` lines the fake Vibe received.
    pub agent_log: PathBuf,
    pub confirmer: FakeConfirmer,
}

/// A core over the scripted fake Vibe with the agent tools on. `tag` separates the fake's logs per core.
pub async fn rig(dirs: &Dirs, tag: &str) -> Rig {
    rig_with(dirs, tag, |_| {}).await
}

/// [`rig`] with a last change to the core's config (the preview engine).
pub async fn rig_with(dirs: &Dirs, tag: &str, tweak: impl FnOnce(&mut CoreConfig)) -> Rig {
    rig_args(dirs, tag, &[], tweak).await
}

/// [`rig_with`] with more fake Vibe arguments (`--status-events`, so a running turn shows in the store).
pub async fn rig_args(
    dirs: &Dirs,
    tag: &str,
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> Rig {
    let agent_log = dirs.root.join(format!("agent-config-{tag}.jsonl"));
    let persist = dirs.persist.to_string_lossy().into_owned();
    let mut args = vec!["--script", "--persist", &persist];
    args.extend_from_slice(extra);
    let mut config = fake_config(tag, &args);
    config.cwd.clone_from(&dirs.root);
    config.vibe_home = dirs.root.join("vibe-home");
    config.extra_env.push((
        "VW_FAKE_LOG_AGENT_CONFIG".to_owned(),
        agent_log.clone().into_os_string(),
    ));
    let mut core_config = CoreConfig::default();
    core_config.procmon.ledger_dir = Some(dirs.ledger.clone());
    core_config.procmon.leases = dirs.leases.clone();
    core_config.procmon.stop.term_grace = Duration::from_millis(1500);
    let mut tools = AgentToolsConfig::new(PathBuf::from(BIN), dirs.data.clone(), None);
    tools.start_wait = Duration::from_secs(6);
    core_config.agent_tools = Some(tools);
    tweak(&mut core_config);
    let confirmer = FakeConfirmer::new();
    let core = Core::start(config, Arc::new(CanaryKey), confirmer.clone(), core_config)
        .await
        .expect("core starts on the fake server");
    Rig {
        core,
        dirs: dirs.clone(),
        agent_log,
        confirmer,
    }
}

impl Rig {
    /// Starts a session in `folder` (picked natively, as the app's folder dialog does), in `agent` mode.
    /// `auto-approve` goes through the native confirmation (the fake confirmer approves), as in the app.
    pub async fn start_session(&self, folder: &Path, agent: &str) -> String {
        self.core.register_picked_folder(folder).unwrap();
        let params = json!({"agentConfig": {"cwd": folder.to_str().unwrap(), "agent": agent}});
        let token = if agent == "auto-approve" {
            let ask: Command = serde_json::from_value(json!({
                "type": "requestConfirmation", "method": "session/start", "params": params
            }))
            .unwrap();
            match self.core.execute(Origin::Ui, ask).await.unwrap() {
                CommandResult::Confirmation { token } => Some(token),
                other => panic!("{other:?}"),
            }
        } else {
            None
        };
        let command: Command = serde_json::from_value(json!({
            "type": "call", "method": "session/start", "params": params, "confirmationToken": token
        }))
        .unwrap();
        match self.core.execute(Origin::Ui, command).await.unwrap() {
            CommandResult::Reply { result, .. } => result["state"]["session"]["id"]
                .as_str()
                .expect("a session id")
                .to_owned(),
            other => panic!("{other:?}"),
        }
    }

    /// A `call` that needs the native confirmation (the fake confirmer approves).
    pub async fn confirmed_call(&self, method: &str, params: Value) -> Value {
        let ask: Command = serde_json::from_value(json!({
            "type": "requestConfirmation", "method": method, "params": params
        }))
        .unwrap();
        let token = match self.core.execute(Origin::Ui, ask).await.unwrap() {
            CommandResult::Confirmation { token } => token,
            other => panic!("{other:?}"),
        };
        let command: Command = serde_json::from_value(json!({
            "type": "call", "method": method, "params": params, "confirmationToken": token
        }))
        .unwrap();
        match self.core.execute(Origin::Ui, command).await.unwrap() {
            CommandResult::Reply { result, .. } => result,
            other => panic!("{other:?}"),
        }
    }

    /// Every `agentConfig` the fake Vibe received.
    pub fn agent_configs(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.agent_log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The `vw` MCP server entry the core injected into the `nth` session-opening request.
    pub fn injected(&self, nth: usize) -> Value {
        let configs = self.agent_configs();
        let config = configs
            .get(nth)
            .unwrap_or_else(|| panic!("no request {nth}: {configs:?}"));
        let servers = config["agentConfig"]["mcpServers"].as_array().unwrap();
        assert_eq!(servers.len(), 1, "exactly one injected server");
        servers[0].clone()
    }

    /// The hook endpoint file the core wrote.
    pub fn endpoint_file(&self) -> PathBuf {
        self.dirs.data.join("hook-endpoint.json")
    }
}

/// The credential and endpoint of an injected server entry.
pub fn credential_of(server: &Value) -> (String, String) {
    (
        server["env"][wire::ENV_ENDPOINT]
            .as_str()
            .unwrap()
            .to_owned(),
        server["env"][wire::ENV_CREDENTIAL]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

/// Calls a tool over the real wire (as the MCP server does).
pub async fn call(who: &(String, String), tool: &str, args: Value) -> Result<Value, String> {
    let (endpoint, credential) = who.clone();
    let tool = tool.to_owned();
    tokio::task::spawn_blocking(move || {
        wire::call(&endpoint, &credential, &tool, args, Duration::from_secs(40))
    })
    .await
    .unwrap()
}

/// A plain HTTP GET on 127.0.0.1; returns the status line.
pub fn http_get(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(18)))
        .ok()?;
    write!(stream, "GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").ok()?;
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).ok()?;
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .map(str::to_owned)
}

/// True when 127.0.0.1:`port` can be bound (nothing listens).
pub fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// A static server command for `process_start` (Python's, bound to loopback, on `$PORT`), in the shell
/// `process_start` uses: `/bin/sh` on Unix, PowerShell on Windows (the Unix form failed there at once,
/// "exec is not recognized", so a test that needs the server listening saw no listener). On Windows the
/// server is a child of PowerShell (no `exec`), so the registered pid is PowerShell's and the listener is
/// attributed through it as a registered ancestor.
pub fn server_command(extra: &str) -> String {
    if cfg!(windows) {
        format!("python -m http.server $env:PORT --bind 127.0.0.1{extra}")
    } else {
        format!("exec python3 -m http.server \"$PORT\" --bind 127.0.0.1{extra}")
    }
}

/// Stops (SIGKILL to the group; `taskkill /T` on Windows) every pid pushed into it when dropped, so a failing
/// test leaves no server.
#[derive(Default)]
pub struct Reaper(pub std::sync::Mutex<Vec<u32>>);

impl Reaper {
    pub fn push(&self, pid: u32) {
        self.0.lock().unwrap().push(pid);
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        #[cfg(unix)]
        for pid in self.0.lock().unwrap().iter() {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(*pid).unwrap());
            let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
        }
        // Windows (): the whole tree (PowerShell and its server), by pid; `taskkill` is in System32.
        #[cfg(windows)]
        for pid in self.0.lock().unwrap().iter() {
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .output();
        }
    }
}

/// The value of `key` in a started-process answer, as a port.
pub fn port_of(answer: &Value) -> u16 {
    u16::try_from(answer["port"].as_u64().unwrap()).unwrap()
}

/// The pid of a started-process answer.
pub fn pid_of(answer: &Value) -> u32 {
    u32::try_from(answer["pid"].as_u64().unwrap()).unwrap()
}

/// Environment helper: `OsString` pairs.
pub fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    pairs
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect()
}
