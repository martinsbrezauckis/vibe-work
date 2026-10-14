//! The core side of the session-scoped agent tools.
//!
//! - **Registration:** every `session/start`, `continue`, `resume` and `fork` that reaches Vibe gets our stdio MCP
//!   server in `agentConfig.mcpServers` ([`AgentTools::inject`], called by `channels` AFTER the gate, which still
//!   refuses a caller's own `mcpServers`), with the session's credential in the server's environment.
//! - **Channel:** a loopback listener (`server`) answers the `vw-agent-tools` binary: the session tools
//!   (`tools`; the secret broker in `secret_run`) for a session credential, `kill_check` (`killcheck`) and `git_check` (`gitcheck`) for the
//!   hook credential.
//! - **Hook endpoint:** the hook's credential and the listener's address in a user-only file the kill guard
//!   reads (`hooks.toml` names the file; the host writes `hooks.toml`).
//!
//! Depends on: vw-agent-tools (credentials, spawn, wire), vw-procmon, vw-platform, tokio. Called by: `core`
//! (start, shutdown), `channels` (injection), `sessions` (revocation on delete), `handoff` (a session id hand-off
//! moves the credential to the new id).
//! Invariants:
//! - No credential is ever logged, put in argv, or returned to a client.
//! - The listener is loopback only and owned by the core (aborted on shutdown).

mod browser_tools;
pub(crate) use browser_tools::attach_tab;
pub(crate) mod copycheck;
mod gitcheck;
mod handbook_tool;
mod killcheck;
mod launches;
pub(crate) mod panel;
mod preview_tools;
mod process_reply;
mod secret_exec;
mod secret_pin;
mod secret_run;
pub(crate) use secret_run::permission_project_of;
mod server;
pub(crate) mod session;
mod start;
mod tools;
mod work_tools;

/// The secret broker's dialog method (one of the vault's native-only dialogs, `vault_scope`).
pub(crate) use secret_run::METHOD as SECRET_RUN_METHOD;

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::JoinHandle;
use vw_agent_tools::credential::{CredentialTable, EntryId};
use vw_agent_tools::hook::EndpointFile;
use vw_agent_tools::spawn::Started;
use vw_agent_tools::wire;
use vw_procmon::ProcessRef;

/// Settings of the agent tools. `None` in `CoreConfig::agent_tools` = no MCP server is registered and no hook
/// endpoint is written (tests without the binary, builds without it).
#[derive(Clone, Debug)]
pub struct AgentToolsConfig {
    /// Absolute path of the `vw-agent-tools` binary (MCP server and hook).
    pub binary: PathBuf,
    /// User-only folder for the hook endpoint file, the launch file and output logs.
    pub data_dir: PathBuf,
    /// The base environment of processes started with `process_start` (the allowlist; `PORT` is added).
    /// Build it with [`AgentToolsConfig::process_env_from`]: never the app's own environment.
    pub process_env: Vec<(OsString, OsString)>,
    /// How long start/restart waits after spawning for a listener or exit. Default 8 s; capped by
    /// the reply budget even when an OS scan stalls. Readiness then comes from `processes_list`.
    pub start_wait: Duration,
    /// Live `process_start` processes per session. Default 8.
    pub max_per_session: usize,
    /// Test switch: called with the program's path after `secret_run`'s final check, right before the
    /// program starts, so a test can race a file swap into exactly that window. Default `None`.
    pub secret_run_before_start: Option<BeforeStart>,
}

/// A test callback for [`AgentToolsConfig::secret_run_before_start`].
#[derive(Clone)]
pub struct BeforeStart(pub Arc<dyn Fn(&Path) + Send + Sync>);

impl std::fmt::Debug for BeforeStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BeforeStart(..)")
    }
}

impl AgentToolsConfig {
    /// Defaults for `binary` and `data_dir`, with the environment built from this process's.
    #[must_use]
    pub fn new(binary: PathBuf, data_dir: PathBuf, home: Option<&Path>) -> Self {
        Self {
            binary,
            data_dir,
            process_env: Self::process_env_from(std::env::vars_os().collect(), home),
            start_wait: Duration::from_secs(8),
            max_per_session: 8,
            secret_run_before_start: None,
        }
    }

    /// The hook endpoint file (the core writes it at start; `hooks.toml` names it).
    #[must_use]
    pub fn endpoint_file(&self) -> PathBuf {
        self.data_dir.join("hook-endpoint.json")
    }

    /// The environment for agent processes: only `vw_supervisor::INHERITED` names, no hijack
    /// variable, no `MISTRAL_API_KEY`, no `VW_*`; `HOME` (and `USERPROFILE` on Windows) replaced by `home` when
    /// given (the same home Vibe's server gets). Git and forge credentials are then stripped/pinned.
    #[must_use]
    pub fn process_env_from(
        parent: Vec<(OsString, OsString)>,
        home: Option<&Path>,
    ) -> Vec<(OsString, OsString)> {
        let allowed = |name: &str| {
            vw_supervisor::INHERITED
                .iter()
                .any(|a| a.eq_ignore_ascii_case(name) && (cfg!(windows) || *a == name))
                && !vw_supervisor::is_hijack_var(name)
                && !name.eq_ignore_ascii_case(vw_supervisor::API_KEY_VAR)
                && !name.to_ascii_uppercase().starts_with("VW_")
        };
        let mut env: Vec<(OsString, OsString)> = parent
            .into_iter()
            .filter(|(k, _)| k.to_str().is_some_and(allowed))
            .collect();
        if let Some(home) = home {
            let names: &[&str] = if cfg!(windows) {
                &["HOME", "USERPROFILE"]
            } else {
                &["HOME"]
            };
            for name in names {
                env.retain(|(k, _)| !k.to_string_lossy().eq_ignore_ascii_case(name));
                env.push((OsString::from(name), home.as_os_str().to_owned()));
            }
        }
        vw_supervisor::isolate_credentials(&mut env);
        env
    }
}

/// A credential minted for one forwarded request, settled when the answer arrives.
#[derive(Clone, Copy)]
pub(crate) enum Ticket {
    /// A start, continue or fork: bind to the session id of the result, or revoke.
    Pending(EntryId),
    /// A resume: the session's own credential, nothing to settle.
    Known,
}

/// What `process_start` started during this run (for exit codes and output after the process ended).
pub(crate) struct StartedHere {
    pub(crate) session_id: String,
    pub(crate) started: Started,
    pub(crate) log: PathBuf,
    pub(crate) command: String,
}

/// A bounded output snapshot retained after the launch log is removed.
pub(crate) struct FinishedHere {
    pub(crate) session_id: String,
    pub(crate) process: ProcessRef,
    pub(crate) status: String,
    pub(crate) command: String,
    pub(crate) shell: String,
    pub(crate) last_output: String,
}

/// The running agent tools.
pub(crate) struct AgentTools {
    pub(crate) config: AgentToolsConfig,
    pub(crate) table: Arc<CredentialTable>,
    pub(crate) endpoint: SocketAddr,
    pub(crate) launches: launches::Launches,
    pub(crate) started: Mutex<HashMap<ProcessRef, StartedHere>>,
    pub(crate) finished: Mutex<std::collections::VecDeque<FinishedHere>>,
    /// `secret_run` requests waiting for the user, running or finished.
    pub(crate) secret_runs: secret_run::SecretRuns,
    /// One approval per agent action (`one_ask`): a yes given in one ask answers the other ask of a call.
    pub(crate) one_ask: crate::one_ask::Ledger,
    /// Which session took which copy (one active session per copy, `copycheck`).
    pub(crate) copy_locks: copycheck::CopyLocks,
    listener: Mutex<Option<std::net::TcpListener>>,
    pub(crate) task: Mutex<Option<JoinHandle<()>>>,
}

impl AgentTools {
    /// Binds the listener, prepares the data folder, loads the launch file and writes the hook endpoint file.
    ///
    /// # Errors
    /// A text reason when the folder, the listener or the random source fails (the core then runs without
    /// agent tools and logs why).
    pub(crate) fn open(config: AgentToolsConfig) -> Result<Self, String> {
        vw_platform::ensure_private_dir(&config.data_dir).map_err(|e| e.to_string())?;
        vw_platform::ensure_private_dir(&config.data_dir.join("output"))
            .map_err(|e| e.to_string())?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let endpoint = listener.local_addr().map_err(|e| e.to_string())?;
        let table = Arc::new(CredentialTable::default());
        let hook = table.hook().map_err(|e| e.to_string())?;
        write_endpoint_file(&config.endpoint_file(), &endpoint, &hook)?;
        let launches = launches::Launches::load(&config.data_dir.join("launches.json"));
        Ok(Self {
            config,
            table,
            endpoint,
            launches,
            started: Mutex::new(HashMap::new()),
            finished: Mutex::new(std::collections::VecDeque::new()),
            secret_runs: secret_run::SecretRuns::default(),
            one_ask: crate::one_ask::Ledger::default(),
            copy_locks: copycheck::CopyLocks::default(),
            listener: Mutex::new(Some(listener)),
            task: Mutex::new(None),
        })
    }

    pub(crate) fn take_listener(&self) -> Option<std::net::TcpListener> {
        self.listener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Restore claims from live launch records before any new lease is handed out (including a server
    /// re-attached while it is still preparing to listen).
    pub(crate) fn restore_ports(&self, monitor: &vw_procmon::ProcMon) {
        let adopted = monitor.inputs(&[]).adopted;
        for launch in self.launches.all() {
            if vw_procmon::is_alive(launch.process)
                && !adopted.iter().any(|a| a.process == launch.process)
            {
                // The launch file is written before the potentially slow ledger scan. A crash in
                // that gap must still restore ownership, listing and safe stop, not just its port.
                let _ = monitor.registry().register(vw_procmon::OwnProcess {
                    session_id: Some(launch.session_id.clone()),
                    process: launch.process,
                    group: vw_procmon::ProcessGroup::None,
                    class: vw_procmon::ProcessClass::Agent,
                    label: start::LABEL.to_owned(),
                    keep_running: false,
                });
            }
            if let Some(port) = launch.port
                && vw_procmon::is_alive(launch.process)
            {
                let _ = monitor
                    .leases()
                    .adopt_process(&launch.session_id, port, launch.process);
            }
        }
    }

    /// The MCP server entry for a credential.
    fn server_entry(&self, credential: &str) -> Value {
        json!({
            "transport": "stdio",
            "name": wire::SERVER_NAME,
            "command": self.config.binary.to_string_lossy(),
            "args": ["mcp"],
            "env": {
                wire::ENV_ENDPOINT: self.endpoint.to_string(),
                wire::ENV_CREDENTIAL: credential,
            },
        })
    }

    /// Adds our MCP server to the `agentConfig` of a session-opening request (after the gate). Returns what to
    /// settle when the answer arrives; `None` for any other method or when no credential could be minted.
    pub(crate) fn inject(&self, method: &str, params: &mut Value) -> Option<Ticket> {
        let (ticket, credential) = match method {
            "session/start" | "session/continue" | "session/fork" => {
                let (entry, credential) = self.table.pending().ok()?;
                (Ticket::Pending(entry), credential)
            }
            "session/resume" => {
                let id = params.get("sessionId").and_then(Value::as_str)?;
                (Ticket::Known, self.table.for_session(id).ok()?)
            }
            _ => return None,
        };
        let object = params.as_object_mut()?;
        let config = object
            .entry("agentConfig")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !config.is_object() {
            *config = Value::Object(serde_json::Map::new());
        }
        if let Some(config) = config.as_object_mut() {
            config.insert(
                "mcpServers".to_owned(),
                Value::Array(vec![self.server_entry(&credential)]),
            );
        }
        Some(ticket)
    }

    /// Settles a ticket: binds a pending credential to the session the result names, or revokes it.
    pub(crate) fn settle(&self, ticket: Ticket, result: Option<&Value>) {
        let Ticket::Pending(entry) = ticket else {
            return;
        };
        match result
            .and_then(|r| r.pointer("/state/session/id"))
            .and_then(Value::as_str)
        {
            Some(id) => self.table.bind(entry, id),
            None => self.table.revoke(entry),
        }
    }

    /// A deleted session's credentials stop working.
    pub(crate) fn session_deleted(&self, session_id: &str) {
        self.table.revoke_session(session_id);
        tools::forget_finished(
            &mut self.finished.lock().unwrap_or_else(PoisonError::into_inner),
            session_id,
        );
    }

    /// Session `old` was handed off to `new` (`handoff.rs`): its credentials, its launch records and the
    /// processes `process_start` started this run now belong to `new`.
    pub(crate) fn session_handed_off(&self, old: &str, new: &str) {
        let moved = self.table.hand_off(old, new);
        self.launches.hand_off(old, new);
        for started in self
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values_mut()
            .filter(|s| s.session_id == old)
        {
            new.clone_into(&mut started.session_id);
        }
        let mut finished = self.finished.lock().unwrap_or_else(PoisonError::into_inner);
        tools::handoff_finished(&mut finished, old, new);
        tracing::debug!(moved, "agent tools: credentials follow the hand-off");
    }

    /// Stops the listener task.
    pub(crate) fn shutdown(&self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl Drop for AgentTools {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Opens the agent tools for a core start (`None` when not configured, or when opening failed: logged, the
/// core then runs without them). Launch records of processes that are gone are dropped.
pub(crate) fn open(
    config: Option<AgentToolsConfig>,
    monitor: &vw_procmon::ProcMon,
) -> Option<Arc<AgentTools>> {
    match AgentTools::open(config?) {
        Ok(tools) => {
            tools.launches.retain_alive(vw_procmon::is_alive);
            tools.restore_ports(monitor);
            Some(Arc::new(tools))
        }
        Err(error) => {
            tracing::warn!(%error, "agent tools are off: they could not be opened");
            None
        }
    }
}

/// Starts the listener task (owned by the tools; aborted by `AgentTools::shutdown`).
pub(crate) fn start_server(shared: &Arc<crate::core::Shared>) {
    if let Some(tools) = &shared.agent_tools
        && let Some(task) = server::spawn_server(shared)
    {
        *tools.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
    }
}

fn write_endpoint_file(path: &Path, endpoint: &SocketAddr, credential: &str) -> Result<(), String> {
    let text = serde_json::to_vec(&EndpointFile {
        endpoint: endpoint.to_string(),
        credential: credential.to_owned(),
    })
    .map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.new");
    let _ = std::fs::remove_file(&temp);
    vw_platform::write_private_new(&temp, &text).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}
