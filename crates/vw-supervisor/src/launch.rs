//! One server start: spawn the process, connect a multiplexer and its lobby channel, run the `initialize` handshake.
//!
//! Depends on: `child` (spawn, tree kill), `env` (environment, key), `handshake`, `config`; vw-rpc.
//! Called by: `supervisor` (first start and every restart).
//! Invariants:
//! - A start that fails after the spawn kills and reaps the process before returning; the error
//!   carries the stderr tail (key redacted), never in its `Display`.
//! - The lobby channel is the first channel (id 1) and is handed out only after its
//!   own handshake; every later channel needs its own handshake too (`open_channel`).

use std::ffi::OsString;
use std::time::{Duration, Instant};

use vw_protocol::ServerInfo;
use vw_rpc::{Connection, Multiplexer, MultiplexerBuilder};

use crate::child::{self, Launch, ServerChild};
use crate::config::{SupervisorConfig, SupervisorError};
use crate::env::{EnvPlan, SecretProvider, build_env};
use crate::handshake::handshake;

/// One running, initialized server.
pub(crate) struct Running {
    /// The lobby channel of the multiplexed process.
    pub(crate) conn: Connection,
    pub(crate) mux: Multiplexer,
    pub(crate) child: ServerChild,
    pub(crate) started: Instant,
    pub(crate) server: ServerInfo,
}

/// Spawns a server (`program`: the canonical executable; `config.program` is only the configured value),
/// connects and runs the handshake.
pub(crate) async fn launch(
    config: &SupervisorConfig,
    program: &std::path::Path,
    secrets: &dyn SecretProvider,
    pid_slot: &std::sync::Mutex<Option<u32>>,
    ids: &crate::live_ids::LiveIds,
) -> Result<Running, SupervisorError> {
    if let Some(check) = &config.before_launch {
        check(program).map_err(SupervisorError::BeforeLaunch)?;
    }
    let key = secrets.mistral_api_key()?;
    let mcp_token = secrets.mcp_forward_token();
    let parent = config
        .parent_env
        .clone()
        .unwrap_or_else(|| std::env::vars_os().collect());
    let env = build_env(&EnvPlan {
        parent,
        extra: &config.extra_env,
        vibe_home: &config.vibe_home,
        home: config.home.as_deref(),
        key: key.as_ref(),
        mcp_token: mcp_token.as_ref(),
    })?;
    crate::shell_env::prepare(&config.vibe_home)?;
    let mut argv = server_argv(config, program);
    let program = argv.remove(0);
    let launch = Launch {
        program: program.clone(),
        args: argv,
        env,
        cwd: config.cwd.clone(),
        watchdog_grace: config.watchdog_grace,
    };
    let (child, pipes) = child::spawn(&launch, ids).map_err(|source| SupervisorError::Spawn {
        program: program.into(),
        source,
    })?;
    *pid_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = child.pid();
    let mut connection = config.connection.clone();
    if let Some(key) = &key {
        connection.stderr_redactions = connection
            .stderr_redactions
            .extended([key.expose().to_owned()]);
    }
    let mut builder = MultiplexerBuilder::new(connection)
        .stderr(pipes.stderr)
        .exit_watch(child.exit_watch());
    for (method, handler) in &config.handlers {
        builder = builder.handler_arc(method.clone(), handler.clone());
    }
    let mux = builder.start(pipes.stdout, pipes.stdin);
    let conn = match mux.open_channel() {
        Ok(lobby) => lobby,
        Err(source) => {
            let exit = child.kill_and_reap(Duration::from_secs(5)).await;
            return Err(SupervisorError::Handshake {
                source: Box::new(source),
                exit,
                stderr_tail: mux.stderr_tail(40),
            });
        }
    };
    match handshake(&conn, &config.initialize, config.handshake_timeout).await {
        Ok(response) => {
            tracing::info!(pid = child.pid(), server = %response.server_info.name, version = %response.server_info.version, "vibe-app-server ready");
            Ok(Running {
                conn,
                mux,
                child,
                started: Instant::now(),
                server: response.server_info,
            })
        }
        Err(source) => {
            let exit = child.kill_and_reap(Duration::from_secs(5)).await;
            // Give the stderr pump a moment to collect the traceback of a crashed start.
            let report = tokio::time::timeout(Duration::from_secs(2), conn.closed())
                .await
                .ok();
            let stderr_tail = report.map_or_else(|| conn.stderr_tail(40), |r| r.stderr_tail);
            Err(SupervisorError::Handshake {
                source: Box::new(source),
                exit,
                stderr_tail,
            })
        }
    }
}

/// Opens one more channel on a multiplexed process and runs its handshake. A channel whose handshake
/// fails is closed again (its server side would otherwise linger).
pub(crate) async fn open_channel(
    mux: &Multiplexer,
    config: &SupervisorConfig,
) -> Result<Connection, SupervisorError> {
    let conn = mux.open_channel().map_err(SupervisorError::Rpc)?;
    match handshake(&conn, &config.initialize, config.handshake_timeout).await {
        Ok(_) => Ok(conn),
        Err(source) => {
            conn.close_input().await;
            Err(SupervisorError::Handshake {
                source: Box::new(source),
                exit: None,
                stderr_tail: mux.stderr_tail(40),
            })
        }
    }
}

/// The child Stop and configuration-switch adapter, passed to the environment's Python with `-c`.
pub const CHILD_STOP_ADAPTER: &str = include_str!("vibe_child_stop.py");

/// The interpreter the server is started with when `program` (canonical) runs through [`CHILD_STOP_ADAPTER`]:
/// the spawned command is then `<python> -P -c <adapter> <program> <mode args>...`, so the spawned process is that
/// Python, not `program`. `None` when `program` itself is spawned. The ownership check (vw-core-api `own_server`)
/// uses the same answer: on Windows the environment's `python.exe` is a launcher that starts the base Python as a
/// child, and that child is the process Vibe reports as a session's lease holder.
#[must_use]
pub fn adapter_python(program: &std::path::Path) -> Option<std::path::PathBuf> {
    // Only Vibe's own server console script runs through the adapter.
    if !matches!(
        program.file_name().and_then(|n| n.to_str()),
        Some("vibe-app-server" | "vibe-app-server.exe")
    ) {
        return None;
    }
    crate::python::server_python(program)
}

/// Build the server command inside any sandbox/recorder launcher.
fn server_argv(config: &SupervisorConfig, program: &std::path::Path) -> Vec<OsString> {
    let mut argv: Vec<OsString> = config.launcher.clone();
    // Run the exact selected environment with our version-pinned adapter; launchers (sandbox/recorder) still wrap
    // the entire server process.
    if let Some(python) = adapter_python(program) {
        // `-P`: `python -c` would put the working folder (the app's `work` folder, where helper chats work) first
        // on the import path, so a planted `json.py` there would run in the server at the next start. The console
        // script it replaces only searched its own `bin`/`Scripts` folder.
        argv.extend([
            python.into_os_string(),
            "-P".into(),
            "-c".into(),
            CHILD_STOP_ADAPTER.into(),
        ]);
    }
    argv.push(program.as_os_str().to_owned());
    argv.push("--multiplex".into());
    argv.extend(config.extra_args.iter().cloned());
    argv
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_adapter_never_imports_from_the_working_folder() {
        let root = std::env::temp_dir().join(format!("vw-server-argv-{}", std::process::id()));
        let bin = root.join("venv/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("vibe-app-server"), "#!/bin/sh\n").unwrap();
        std::fs::write(bin.join("python"), "").unwrap();
        let program = bin.join("vibe-app-server");
        let config = SupervisorConfig::new(program.clone(), root.join("home"), root.join("work"));
        let argv = server_argv(&config, &program);
        let python = std::fs::canonicalize(&bin).unwrap().join("python");
        assert_eq!(argv[0], python.as_os_str());
        assert_eq!(argv[1], "-P");
        assert_eq!(argv[2], "-c");
        assert_eq!(argv[4], program.as_os_str());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
