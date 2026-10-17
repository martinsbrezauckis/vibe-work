//! One server start: spawn the process, connect (a plain connection or a multiplexer plus its lobby
//! channel), run the `initialize` handshake.
//!
//! Depends on: `child` (spawn, tree kill), `env` (environment, key), `handshake`, `config`; vw-rpc.
//! Called by: `supervisor` (first start and every restart).
//! Invariants:
//! - A start that fails after the spawn kills and reaps the process before returning; the error
//!   carries the stderr tail (key redacted), never in its `Display`.
//! - In multiplex mode the lobby channel is the first channel (id 1) and is handed out only after its
//!   own handshake; every later channel needs its own handshake too (`open_channel`).

use std::ffi::OsString;
use std::time::{Duration, Instant};

use vw_protocol::ServerInfo;
use vw_rpc::{Connection, ConnectionBuilder, Multiplexer, MultiplexerBuilder};

use crate::child::{self, Launch, ServerChild};
use crate::config::{ServerMode, SupervisorConfig, SupervisorError};
use crate::env::{EnvPlan, SecretProvider, build_env};
use crate::handshake::handshake;

/// How this process is connected.
pub(crate) enum Link {
    /// One plain stdio connection (`ServerMode::Single`).
    Single,
    /// A multiplexed process; `Running::conn` is its lobby channel.
    Multiplex(Multiplexer),
}

/// One running, initialized server.
pub(crate) struct Running {
    /// The plain connection, or the lobby channel of a multiplexed process.
    pub(crate) conn: Connection,
    pub(crate) link: Link,
    pub(crate) child: ServerChild,
    pub(crate) started: Instant,
    pub(crate) server: ServerInfo,
}

impl Running {
    /// The multiplexer, in multiplex mode.
    pub(crate) fn mux(&self) -> Option<&Multiplexer> {
        match &self.link {
            Link::Multiplex(mux) => Some(mux),
            Link::Single => None,
        }
    }
}

/// Spawns a server (`program`: the canonical executable; `config.program` is only the configured value),
/// connects and runs the handshake.
pub(crate) async fn launch(
    config: &SupervisorConfig,
    program: &std::path::Path,
    secrets: &dyn SecretProvider,
    pid_slot: &std::sync::Mutex<Option<u32>>,
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
    let mut argv: Vec<OsString> = config.launcher.clone();
    argv.push(program.as_os_str().to_owned());
    argv.extend(config.mode.args());
    argv.extend(config.extra_args.iter().cloned());
    let program = argv.remove(0);
    let launch = Launch {
        program: program.clone(),
        args: argv,
        env,
        cwd: config.cwd.clone(),
        watchdog_grace: config.watchdog_grace,
    };
    let (child, pipes) = child::spawn(&launch).map_err(|source| SupervisorError::Spawn {
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
    let (conn, link) = match config.mode {
        ServerMode::Single => {
            let mut builder = ConnectionBuilder::new(connection)
                .stderr(pipes.stderr)
                .exit_watch(child.exit_watch());
            for (method, handler) in &config.handlers {
                builder = builder.handler_arc(method.clone(), handler.clone());
            }
            (builder.start(pipes.stdout, pipes.stdin), Link::Single)
        }
        ServerMode::Multiplex => {
            let mut builder = MultiplexerBuilder::new(connection)
                .stderr(pipes.stderr)
                .exit_watch(child.exit_watch());
            for (method, handler) in &config.handlers {
                builder = builder.handler_arc(method.clone(), handler.clone());
            }
            let mux = builder.start(pipes.stdout, pipes.stdin);
            match mux.open_channel() {
                Ok(lobby) => (lobby, Link::Multiplex(mux)),
                Err(source) => {
                    let exit = child.kill_and_reap(Duration::from_secs(5)).await;
                    return Err(SupervisorError::Handshake {
                        source: Box::new(source),
                        exit,
                        stderr_tail: mux.stderr_tail(40),
                    });
                }
            }
        }
    };
    match handshake(&conn, &config.initialize, config.handshake_timeout).await {
        Ok(response) => {
            tracing::info!(pid = child.pid(), server = %response.server_info.name, version = %response.server_info.version, mode = ?config.mode, "vibe-app-server ready");
            Ok(Running {
                conn,
                link,
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
