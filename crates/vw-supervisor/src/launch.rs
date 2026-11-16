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
use vw_platform::sync::LockRecover;

use vw_protocol::ServerInfo;
use vw_rpc::{Connection, Multiplexer, MultiplexerBuilder};

use crate::child::{self, Launch, ServerChild};
use crate::config::{SupervisorConfig, SupervisorError};
use crate::env::{EnvPlan, SecretProvider, build_env};
use crate::handshake::handshake;

#[cfg(test)]
#[path = "launch_command_tests.rs"]
mod command_tests;

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
/// connects and runs the handshake, waiting up to `handshake_timeout` for its answer.
pub(crate) async fn launch(
    config: &SupervisorConfig,
    program: &std::path::Path,
    handshake_timeout: Duration,
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
    let mut argv = server_argv(config, program)?;
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
    *pid_slot.lock_or_recover() = child.pid();
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
    match handshake(&conn, &config.initialize, handshake_timeout).await {
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

/// The Stop, configuration-switch, plugin-hook and remote-cache guards, delivered through a private adapter file. The shared
/// guarded-version list comes first: a Vibe outside it stops the start with [`GUARD_REFUSAL`]'s message.
pub const CHILD_STOP_ADAPTER: &str = concat!(
    include_str!("vibe_guard_versions.py"),
    "\n",
    include_str!("vibe_process_identity.py"),
    "\n",
    include_str!("vibe_process_tracking.py"),
    "\n",
    include_str!("vibe_process_output.py"),
    "\n",
    include_str!("vibe_plugin_enablement.py"),
    "\n",
    include_str!("vibe_mcp_projection.py"),
    "\n",
    include_str!("vibe_skill_cleanup.py"),
    "\n",
    include_str!("vibe_resume_guard.py"),
    "\n",
    include_str!("vibe_keychain_guard.py"),
    "\n",
    include_str!("vibe_remote_cache_guard.py"),
    "\n",
    include_str!("vibe_child_stop.py")
);

/// The Vibe releases the launcher guards were checked against, read from the same `vibe_guard_versions.py` line the
/// guards use (`GUARDED_VIBE_VERSIONS = frozenset({"3.1.2", ...})`), so the update offer and the launcher share one
/// list. A line that does not read as plain quoted versions gives an empty list: nothing is offered (fail closed).
#[must_use]
pub fn guarded_vibe_versions() -> Vec<String> {
    parse_guarded(include_str!("vibe_guard_versions.py"))
}

fn parse_guarded(source: &str) -> Vec<String> {
    let Some(items) = source.lines().find_map(|line| {
        line.strip_prefix("GUARDED_VIBE_VERSIONS = frozenset({")
            .and_then(|rest| rest.strip_suffix("})"))
    }) else {
        return Vec::new();
    };
    let plain = |v: &str| {
        !v.is_empty()
            && v.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let versions: Option<Vec<String>> = items
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .filter(|v| plain(v))
                .map(str::to_owned)
        })
        .collect();
    versions.unwrap_or_default()
}

/// The first words of the launcher's refusal of a Vibe outside the guarded-version list (`vibe_guard_versions.py`
/// `GUARD_REFUSAL`; a test keeps the two equal).
pub const GUARD_REFUSAL: &str = "Vibe Work does not support Vibe ";

/// Why a start failed, for the person: the launcher's own refusal line when the installed Vibe is not one the guards
/// were checked against (taken from the server output only by its fixed first words, at most 300 characters), else
/// the error itself.
#[must_use]
pub fn start_failure(error: &SupervisorError) -> String {
    if let SupervisorError::Handshake { stderr_tail, .. } = error
        && let Some(line) = stderr_tail
            .iter()
            .rev()
            .map(|line| line.trim())
            .find(|line| line.starts_with(GUARD_REFUSAL))
    {
        return line.chars().take(300).collect();
    }
    error.to_string()
}

/// The server's last stderr lines a failed start kept (a handshake that failed), else none. Sensitive: never logged.
#[must_use]
pub fn start_failure_stderr(error: &SupervisorError) -> Vec<String> {
    match error {
        SupervisorError::Handshake { stderr_tail, .. } => stderr_tail.clone(),
        _ => Vec::new(),
    }
}

/// The reason a start failed, as the setup screen shows it: the refusal of an unsupported Vibe stands alone (it is
/// already a whole sentence that says what to do), anything else reads "Vibe did not start: <why>".
#[must_use]
pub fn start_failure_message(error: &SupervisorError) -> String {
    let reason = start_failure(error);
    if reason.starts_with(GUARD_REFUSAL) {
        reason
    } else {
        format!("Vibe did not start: {reason}")
    }
}

/// The interpreter the server is started with when `program` (canonical) runs through [`CHILD_STOP_ADAPTER`]:
/// the spawned command uses a fixed-size isolated loader and a verified adapter file, so the spawned process is that
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
fn server_argv(
    config: &SupervisorConfig,
    program: &std::path::Path,
) -> Result<Vec<OsString>, SupervisorError> {
    let mut argv: Vec<OsString> = config.launcher.clone();
    // Run the exact selected environment with our version-pinned adapter; launchers (sandbox/recorder) still wrap
    // the entire server process.
    if let Some(python) = adapter_python(program) {
        let adapter = crate::adapter_file::prepare(&config.vibe_home)?;
        // -I excludes cwd, script directory, user site and PYTHON* overrides. -u preserves the
        // unbuffered RPC pipes: isolated Python ignores the previous PYTHONUNBUFFERED setting.
        // Stdin remains the multiplexed RPC input, so it cannot carry the adapter source.
        argv.extend([
            python.into_os_string(),
            "-I".into(),
            "-u".into(),
            "-c".into(),
            crate::ADAPTER_BOOTSTRAP.into(),
            adapter.into_os_string(),
            crate::adapter_digest().into(),
        ]);
    }
    argv.push(program.as_os_str().to_owned());
    argv.push("--multiplex".into());
    argv.extend(config.extra_args.iter().cloned());
    Ok(argv)
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
        let argv = server_argv(&config, &program).unwrap();
        let python = std::fs::canonicalize(&bin).unwrap().join("python");
        assert_eq!(argv[0], python.as_os_str());
        assert_eq!(argv[1], "-I");
        assert_eq!(argv[2], "-u");
        assert_eq!(argv[7], program.as_os_str());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
