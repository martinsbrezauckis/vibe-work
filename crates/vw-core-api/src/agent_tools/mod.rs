//! The core side of the session-scoped agent tools.
//!
//! - **Registration:** every `session/start`, `continue`, `resume` and `fork` that reaches Vibe gets our stdio MCP
//!   server in `agentConfig.mcpServers` ([`AgentTools::inject`], called by `channels` AFTER the gate, which still
//!   refuses a caller's own `mcpServers`), with the session's credential in the server's environment, and the
//!   tool filter of the tool sets that load only when needed (`on_demand`).
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
mod secret_dialog;
pub(crate) mod secret_env;
mod secret_exec;
mod secret_run;
pub(crate) use secret_run::{permission_project_of, visible_logins};
mod document_consent;
#[cfg(test)]
mod document_read_tests;
mod document_tools;
mod prepare;
mod search_tool;
mod server;
pub(crate) mod session;
mod session_facts;
pub(crate) mod shell_commands;
mod start;
mod tools;
pub(crate) use prepare::prepare;
mod work_tools;

/// The secret broker's dialog method (one of the vault's native-only dialogs, `vault_scope`).
pub(crate) use secret_run::METHOD as SECRET_RUN_METHOD;

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
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
    /// How long one `propose_protected_change` or `protected_change_status` call waits for the user's decision
    /// before it answers Waiting (the review stays open). Default 40 s: below the tools program's call budget and
    /// the time Vibe allows one session MCP tool call.
    pub decision_wait: Duration,
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
            decision_wait: crate::protected_changes::DECISION_WAIT,
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
    pub(crate) shell_commands: shell_commands::Commands,
    pub(crate) tool_asks: crate::ask::tool_calls::Calls,
    /// Which session took which copy (one active session per copy, `copycheck`).
    pub(crate) copy_locks: copycheck::CopyLocks,
    /// The tool sets that load only when needed and which sessions turned them on (`on_demand`).
    pub(crate) on_demand: crate::on_demand::OnDemand,
    chrome_package: bool,
    /// The core this server belongs to (set by [`start_server`]): each chat's launch reads Chrome Control and the
    /// chat's mode from it (`chrome_launch`).
    shared: OnceLock<Weak<crate::core::Shared>>,
    listener: Mutex<Option<std::net::TcpListener>>,
    pub(crate) task: Mutex<Option<JoinHandle<()>>>,
}

impl AgentTools {
    /// Binds the listener, prepares the data folder, loads the launch file and writes the hook endpoint file.
    ///
    /// # Errors
    /// A text reason when the folder, the listener or the random source fails (the core then runs without
    /// agent tools and logs why).
    pub(crate) fn open(config: AgentToolsConfig, chrome_package: bool) -> Result<Self, String> {
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
            chrome_package,
            shared: OnceLock::new(),
            config,
            table,
            endpoint,
            launches,
            started: Mutex::new(HashMap::new()),
            finished: Mutex::new(std::collections::VecDeque::new()),
            secret_runs: secret_run::SecretRuns::default(),
            one_ask: crate::one_ask::Ledger::default(),
            shell_commands: shell_commands::Commands::default(),
            tool_asks: crate::ask::tool_calls::Calls::default(),
            copy_locks: copycheck::CopyLocks::default(),
            on_demand: crate::on_demand::OnDemand::default(),
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

    /// The MCP server entry for a credential. Vibe's session protocol accepts exactly `transport`, `name`, `command`,
    /// `args`, `env` and `cwd` here and refuses the whole session-opening request for any other field; it applies its
    /// default tool-call time to this server, so every tool answers within it (a long user decision answers Waiting
    /// and is collected with a status tool).
    fn server_entry(&self, credential: &str, chrome: ChromeLaunch) -> Value {
        json!({
            "transport": "stdio",
            "name": wire::SERVER_NAME,
            "command": self.config.binary.to_string_lossy(),
            "args": server_args(chrome, crate::version()),
            "env": {
                wire::ENV_ENDPOINT: self.endpoint.to_string(),
                wire::ENV_CREDENTIAL: credential,
            },
        })
    }

    /// Whether this launch of a chat's tool server advertises the Chrome tools, decided now from the package,
    /// Chrome Control and the mode the request names (`agentConfig.agent`), else the mode the core knows for the
    /// session it opens or continues. Vibe keeps a server's tool list under a hash of its launch line, so the
    /// decision is in the launch line and the list itself never depends on the chat (`gateway::list`).
    fn chrome_launch(&self, params: &Value) -> ChromeLaunch {
        let Some(shared) = self.shared.get().and_then(Weak::upgrade) else {
            // Not attached to a core yet (no session can open before `start_server`): the package decides.
            return if self.chrome_package {
                ChromeLaunch::Catalog
            } else {
                ChromeLaunch::NoPackage
            };
        };
        let mode = params
            .pointer("/agentConfig/agent")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .and_then(|id| crate::chrome_control::check::mode(&shared, id))
            });
        chrome_launch(
            shared.chrome_control.config.available(),
            shared.chrome_control.enabled(),
            mode.as_deref(),
        )
    }

    /// Adds our MCP server to the `agentConfig` of a session-opening request (after the gate), and the tool filter
    /// of every on-demand tool set that is not on for the session (`on_demand`: a new session, a continue or a fork
    /// gets them all). Returns what to settle when the answer arrives; `None` for any other method or when no
    /// credential could be minted.
    pub(crate) fn inject(&self, method: &str, params: &mut Value) -> Option<Ticket> {
        let (ticket, credential, opened) = match method {
            "session/start" | "session/continue" | "session/fork" => {
                let (entry, credential) = self.table.pending().ok()?;
                (Ticket::Pending(entry), credential, None)
            }
            "session/resume" => {
                let id = params.get("sessionId").and_then(Value::as_str)?;
                (
                    Ticket::Known,
                    self.table.for_session(id).ok()?,
                    Some(id.to_owned()),
                )
            }
            _ => return None,
        };
        let chrome = self.chrome_launch(params);
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
                Value::Array(vec![self.server_entry(&credential, chrome)]),
            );
            let patterns = self.on_demand.disabled_patterns(opened.as_deref());
            crate::on_demand::add_patterns(config, &patterns);
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
        self.on_demand.forget(session_id);
        tools::forget_finished(
            &mut self.finished.lock().unwrap_or_else(PoisonError::into_inner),
            session_id,
        );
    }

    /// Session `old` was handed off to `new` (`handoff.rs`): its credentials, its launch records and the
    /// processes `process_start` started this run now belong to `new`.
    pub(crate) fn session_handed_off(&self, old: &str, new: &str) {
        let moved = self.table.hand_off(old, new);
        self.on_demand.hand_off(old, new);
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

/// What a chat's tool server is launched to advertise for Chrome: one launch line each (`server_args`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChromeLaunch {
    /// The Chrome tools (`--chrome-catalog`).
    Catalog,
    /// The package is there but this chat gets no Chrome tools (Chrome Control off, or a mode without a browser).
    Hidden,
    /// This build has no Chrome package (`--without-chrome`).
    NoPackage,
}

/// The Chrome tools are offered to a chat only with the package, with Chrome Control on and when its mode is not
/// known to be one without a browser (Plan, Explore, a custom profile). A mode not known yet (a new chat on the
/// default profile) gets them. Every Chrome call is still authorized against the current mode (`check::authorize`).
fn chrome_launch(package: bool, enabled: bool, mode: Option<&str>) -> ChromeLaunch {
    if !package {
        ChromeLaunch::NoPackage
    } else if enabled && mode.is_none_or(|mode| vw_ask::chrome::Tier::from_mode(mode).is_some()) {
        ChromeLaunch::Catalog
    } else {
        ChromeLaunch::Hidden
    }
}

/// The tool server's launch arguments. Vibe keeps a stdio server's tool list for up to a day, shared by every chat,
/// under a hash of its entry without the environment values: every argument set must therefore have one tool list.
/// The Chrome choice is an argument (the server's list follows it), and so is the app version: after an update the
/// agent gets this build's tools, not the previous build's. Two builds with the same version share the key.
fn server_args(chrome: ChromeLaunch, app_version: &str) -> Vec<String> {
    let mut args = vec!["mcp".to_owned()];
    match chrome {
        ChromeLaunch::Catalog => args.push(wire::CHROME_CATALOG_ARG.to_owned()),
        // The server forwards a Chrome call, which the core refuses with this chat's reason.
        ChromeLaunch::Hidden => {}
        ChromeLaunch::NoPackage => args.push(wire::WITHOUT_CHROME_ARG.to_owned()),
    }
    args.extend([wire::APP_VERSION_ARG.to_owned(), app_version.to_owned()]);
    args
}

/// Opens the agent tools for a core start (`None` when not configured, or when opening failed: logged, the
/// core then runs without them). Launch records of processes that are gone are dropped.
pub(crate) fn open(
    config: Option<AgentToolsConfig>,
    monitor: &vw_procmon::ProcMon,
    chrome_package: bool,
) -> Option<Arc<AgentTools>> {
    match AgentTools::open(config?, chrome_package) {
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
    if let Some(tools) = &shared.agent_tools {
        let _ = tools.shared.set(Arc::downgrade(shared));
    }
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

#[cfg(test)]
mod server_args_tests {
    use super::{ChromeLaunch, chrome_launch, server_args};

    /// Vibe keeps a stdio server's tool list for a day under a fingerprint of its entry without the environment
    /// values, so an update must change the arguments or the agent keeps the previous build's tools.
    #[test]
    fn each_app_version_launches_the_tool_server_with_its_own_arguments() {
        for chrome in [
            ChromeLaunch::Catalog,
            ChromeLaunch::Hidden,
            ChromeLaunch::NoPackage,
        ] {
            assert_ne!(server_args(chrome, "0.1.4"), server_args(chrome, "0.1.5"));
            assert_eq!(server_args(chrome, "0.1.5"), server_args(chrome, "0.1.5"));
        }
    }

    /// Before: the arguments were the same for every chat of a build while the Chrome list differed per chat, so a
    /// chat with Chrome Control off seeded an empty Chrome list that later chats got from Vibe's cache. The Chrome
    /// choice is now made per chat in the arguments; each choice has its own launch line.
    #[test]
    fn the_chrome_choice_is_in_the_launch_line() {
        for (package, enabled, mode, expected) in [
            (true, false, Some("ask"), ChromeLaunch::Hidden),
            (true, true, Some("ask"), ChromeLaunch::Catalog),
            (true, true, Some("accept-edits"), ChromeLaunch::Catalog),
            (true, true, Some("auto-approve"), ChromeLaunch::Catalog),
            (true, true, Some("vw-plan"), ChromeLaunch::Hidden),
            (true, true, Some("plan"), ChromeLaunch::Hidden),
            (true, true, Some("explore"), ChromeLaunch::Hidden),
            (true, true, None, ChromeLaunch::Catalog),
            (true, false, None, ChromeLaunch::Hidden),
            (false, true, Some("ask"), ChromeLaunch::NoPackage),
            (false, true, None, ChromeLaunch::NoPackage),
            (false, false, Some("vw-plan"), ChromeLaunch::NoPackage),
        ] {
            assert_eq!(
                chrome_launch(package, enabled, mode),
                expected,
                "{package} {enabled} {mode:?}"
            );
        }
        let lines = [
            server_args(ChromeLaunch::Catalog, "0.1.5"),
            server_args(ChromeLaunch::Hidden, "0.1.5"),
            server_args(ChromeLaunch::NoPackage, "0.1.5"),
        ];
        assert_eq!(
            lines[0],
            ["mcp", "--chrome-catalog", "--app-version", "0.1.5"]
        );
        assert_eq!(lines[1], ["mcp", "--app-version", "0.1.5"]);
        assert_eq!(
            lines[2],
            ["mcp", "--without-chrome", "--app-version", "0.1.5"]
        );
    }
}
