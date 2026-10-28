//! Real Vibe, scripted models and agent-tools integration helpers for disposable tests.
//! They start test binaries and call the local tool wire; callers own all processes and folders.
#![allow(dead_code, unreachable_pub)]

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use vw_agent_tools::wire;
use vw_core_api::{AgentToolsConfig, Command, CommandResult, Core, CoreConfig, Origin};
use vw_procmon::LeaseConfig;

mod launch_reaper;
#[allow(unused_imports)]
pub use launch_reaper::LaunchReaper;

use crate::common::gate::FakeConfirmer;
use crate::common::processes::scratch;

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
    pub stored: PathBuf,
    pub leases: LeaseConfig,
}

impl Dirs {
    pub fn new(name: &str) -> Self {
        let root = scratch(name);
        Self {
            ledger: root.join("ledger"),
            data: root.join("agent-tools"),
            stored: root.join("vibe-home/logs/session/unified"),
            leases: per_run_leases(),
            root,
        }
    }
}

pub struct Rig {
    pub core: Core,
    pub dirs: Dirs,
    /// `{"method", "agentConfig"}` lines the real Vibe received.
    pub agent_log: PathBuf,
    /// `{"method", "params"}` of every request the real Vibe received, in arrival order.
    pub params_log: PathBuf,
    pub confirmer: FakeConfirmer,
}

/// Tool requests formerly hidden in protocol fixtures now come from the loopback model.
fn model_script() -> crate::common::scripted_model::Script {
    serde_json::from_value(json!({"markers":true,"rules":[
        {"system":"You are a security classifier","user":"smart-safe","reply":{"text":"{\"safe\":true,\"risk_level\":\"low\",\"user_authorization\":\"high\",\"reason\":\"Scripted safe case\"}"}},
        {"system":"You are a security classifier","reply":{"text":"{\"safe\":false,\"risk_level\":\"high\",\"user_authorization\":\"high\",\"reason\":\"Scripted approval case\"}"}},
        {"side":true,"reply":{"text":"Scripted title"}},
        {"user":"^(Fill in the form on my supplier portal in my Chrome[.]|Open my Chrome and check the order[.])$","step":0,"reply":{"tools":[{"name":"write_file","args":{"path":"fixture.txt","content":"scripted tool\n"}}]}},
        {"user":"^write a file$","step":0,"reply":{"tools":[{"name":"write_file","args":{"path":"fixture.txt","content":"scripted tool\n"}}]}},
        {"user":"^Explain the result[.]$","reply":{"text":"approved: approve"}},
        {"user":"^read it to me$","step":0,"reply":{"text":"Sure, reading it now.","tools":[{"name":"run_typescript","args":{"code":"async function main() { return await tools.mcp_vw.read_aloud({}); }"}}]}},
        {"reply":{"text":"Scripted reply"}}
    ]})).unwrap()
}

/// A core over real Vibe with the agent tools on. `tag` separates the process logs per core.
pub async fn rig(dirs: &Dirs, tag: &str) -> Rig {
    rig_with(dirs, tag, |_| {}).await
}

/// [`rig`] with a last change to the core's config (the preview engine).
pub async fn rig_with(dirs: &Dirs, tag: &str, tweak: impl FnOnce(&mut CoreConfig)) -> Rig {
    rig_args(dirs, tag, &[], tweak).await
}

/// [`rig_with`] with a strict empty server-options boundary; real Vibe publishes its own status events.
pub async fn rig_args(
    dirs: &Dirs,
    tag: &str,
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> Rig {
    assert!(extra.is_empty(), "use real faults: {extra:?}");
    rig_with_script(dirs, tag, model_script(), tweak).await
}

/// [`rig_with`] with explicit model replies, including holds controlled by a disposable file.
pub async fn rig_with_script(
    dirs: &Dirs,
    tag: &str,
    script: crate::common::scripted_model::Script,
    tweak: impl FnOnce(&mut CoreConfig),
) -> Rig {
    let run = dirs.root.join(tag);
    std::fs::create_dir_all(&run).unwrap();
    let model = crate::common::scripted_model::ScriptedModel::start(script).await;
    let mut config = crate::common::shared_real_vibe::supervisor_config(&run, &model);
    crate::common::shared_real_vibe::reuse_home(&mut config, &dirs.root.join("vibe-home"), &model);
    config.cwd.clone_from(&dirs.root);
    crate::common::core::record(&mut config, &run);
    let agent_log = run.join("agent-config.jsonl");
    let params_log = run.join("params.jsonl");
    let mut core_config = CoreConfig::default();
    core_config.procmon.ledger_dir = Some(dirs.ledger.clone());
    core_config.procmon.leases = dirs.leases.clone();
    core_config.procmon.stop.term_grace = Duration::from_millis(1500);
    let mut tools = AgentToolsConfig::new(PathBuf::from(BIN), dirs.data.clone(), None);
    tools.start_wait = Duration::from_secs(6);
    core_config.agent_tools = Some(tools);
    tweak(&mut core_config);
    std::fs::write(
        config.vibe_home.join("hooks.toml"),
        vw_host::agent_tools::hook_block(
            &core_config.agent_tools.as_ref().unwrap().binary,
            &core_config.agent_tools.as_ref().unwrap().endpoint_file(),
        ),
    )
    .unwrap();
    let confirmer = FakeConfirmer::new();
    let secret = model.route(&mut config).await;
    let core = Core::start(config, secret, confirmer.clone(), core_config)
        .await
        .expect("core starts on real pinned Vibe");
    Rig {
        core,
        dirs: dirs.clone(),
        agent_log,
        params_log,
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
            CommandResult::Reply { result, .. } => {
                let id = result["state"]["session"]["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                self.bind_hook(&id);
                id
            }
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
            CommandResult::Reply { result, .. } => {
                if let Some(id) = result["state"]["session"]["id"].as_str() {
                    self.bind_hook(id);
                }
                result
            }
            other => panic!("{other:?}"),
        }
    }

    pub fn bind_hook(&self, session: &str) {
        let server = self.injected(self.agent_configs().len() - 1);
        let who = credential_of(&server);
        bind_session(&self.core, &self.endpoint_file(), who, session);
    }

    /// Every `agentConfig` the real Vibe received.
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

type HookRoutes = std::collections::HashMap<(String, String), (String, String, Core)>;
fn hook_routes() -> &'static std::sync::Mutex<HookRoutes> {
    static ROUTES: std::sync::OnceLock<std::sync::Mutex<HookRoutes>> = std::sync::OnceLock::new();
    ROUTES.get_or_init(Default::default)
}

/// Associates a manually created session with its actual hook endpoint for direct tool fixtures.
pub fn bind_session(core: &Core, file: &Path, who: (String, String), session: &str) {
    let hook: vw_agent_tools::hook::EndpointFile =
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    hook_routes()
        .lock()
        .unwrap()
        .insert(who, (hook.credential, session.into(), core.clone()));
}

/// Calls the hook and then the tool over the real wire. Direct policy fixtures answer the generic
/// access-mode card once as the user; model-driven tests exercise that card without this helper.
pub async fn call(who: &(String, String), tool: &str, args: Value) -> Result<Value, String> {
    let args = prepare_call(who, tool, args).await?;
    raw_call(who, tool, args).await
}

/// Completes hook policy before a test controls the tool's own wire connection.
pub async fn prepare_call(
    who: &(String, String),
    tool: &str,
    mut args: Value,
) -> Result<Value, String> {
    let route = hook_routes().lock().unwrap().get(who).cloned();
    if args.get(wire::tools::RECEIPT_FIELD).is_none()
        && let Some((hook, session, core)) = route
    {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let call = format!(
            "fixture-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let payload = json!({"session_id":session,"tool_call_id":call,"tool_name":format!("mcp_vw.{tool}"),"tool_input":args});
        let endpoint = who.0.clone();
        let mut pending = tokio::task::spawn_blocking(move || {
            wire::call(&endpoint, &hook, wire::tools::TOOL_RECEIPT, payload, None)
        });
        args = loop {
            tokio::select! {
                result = &mut pending => break result.unwrap()?,
                () = tokio::time::sleep(Duration::from_millis(10)) => {
                    let state = core.execute(Origin::Ui, Command::ReadSession { session_id:session.clone() }).await;
                    if let Ok(CommandResult::Session { snapshot, .. }) = state {
                        for entry in snapshot.state["history"].as_array().into_iter().flatten() {
                            if entry["vwSourceCallId"] == call && entry["state"]["status"] == "open" {
                                let id = format!("callback:{}", entry["id"].as_str().unwrap());
                                core.answer_ask(&id, serde_json::from_value(json!({"answer":"once","note":null})).unwrap()).await.unwrap();
                            }
                        }
                    }
                }
            }
        };
    }
    Ok(args)
}

/// Sends a raw tool request for tests of credential and decision rejection.
pub async fn raw_call(who: &(String, String), tool: &str, args: Value) -> Result<Value, String> {
    let (endpoint, credential) = who.clone();
    let tool = tool.to_owned();
    tokio::task::spawn_blocking(move || {
        wire::call(&endpoint, &credential, &tool, args, Duration::from_secs(40))
    })
    .await
    .unwrap()
}

/// A plain HTTP GET on 127.0.0.1; returns the complete status line after draining the response.
/// Closing early resets Python's body write and floods its output tail with a traceback.
pub fn http_get(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(18)))
        .ok()?;
    write!(stream, "GET / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").ok()?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).ok()?;
    std::io::copy(&mut reader, &mut std::io::sink()).ok()?;
    Some(status.trim_end().to_owned())
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

/// Exercise the new secret broker start/status protocol, retaining the old tests' final-result assertions.
/// Every wire call uses the real core; the command is submitted exactly once.
#[allow(dead_code)]
pub async fn call_and_collect(
    who: &(String, String),
    tool: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let before = std::time::Instant::now();
    let mut value = call(who, tool, args).await?;
    if tool != "secret_run" {
        return Ok(value);
    }
    assert!(
        before.elapsed() < std::time::Duration::from_secs(30),
        "secret_run must return promptly"
    );
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while let Some(id) = value["requestId"].as_str() {
        assert!(
            tokio::time::Instant::now() < until,
            "secret result did not finish: {value}"
        );
        value = call(
            who,
            "secret_run_status",
            serde_json::json!({"requestId":id}),
        )
        .await?;
    }
    Ok(value)
}
