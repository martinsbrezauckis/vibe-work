//! Supervisor configuration, its errors and its observable state.
//!
//! Depends on: vw-rpc, vw-protocol, `backoff`, `env`. Called by: the app (builds a config from
//! settings), tests, and `supervisor`.
//! Invariant: nothing here holds the API key; the key comes from the `SecretProvider` at each
//! start.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use vw_protocol::{InitializeParams, ServerInfo};
use vw_rpc::{ConnectionConfig, DeathReport, ExitInfo, RpcError, ServerRequestHandler};

use crate::backoff::Backoff;
use crate::env::{EnvError, SecretError};
use crate::handshake::default_initialize_params;

/// How the server multiplexes sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServerMode {
    /// One stdio connection per process, with one attached session for the whole process. The app
    /// uses a channel per session instead; kept for tests that
    /// need a plain connection (`Supervisor::current` is then that connection).
    Single,
    /// `--multiplex` (the default): many isolated channels in one process, each its own
    /// server with its own attached session, events and callbacks (Vibe `app_server/multiplex.py`). `Supervisor::current` is the "lobby" channel; `Supervisor::open_channel` opens more.
    /// Started as `--multiplex --experimental-harness`: multiplexed stdio runs on the unified
    /// harness, which the second flag selects explicitly (`stdio.py:49`).
    #[default]
    Multiplex,
}

impl ServerMode {
    /// Server arguments this mode needs.
    #[must_use]
    pub fn args(self) -> Vec<OsString> {
        match self {
            Self::Single => Vec::new(),
            Self::Multiplex => vec!["--multiplex".into(), "--experimental-harness".into()],
        }
    }
}

type LaunchCheck = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

/// Everything needed to run and restart one `vibe-app-server`.
pub struct SupervisorConfig {
    /// The server executable (from `Discovery`). Must be absolute; `Supervisor::start`
    /// canonicalizes it and spawns exactly that path.
    pub program: PathBuf,
    /// Optional launcher argv placed before the program (e.g. a bubblewrap sandbox in tests).
    pub launcher: Vec<OsString>,
    /// Extra server arguments.
    pub extra_args: Vec<OsString>,
    /// Session multiplexing mode (default [`ServerMode::Multiplex`]).
    pub mode: ServerMode,
    /// `VIBE_HOME` for the server. Tests and dev runs use their own homes, never `~/.vibe`.
    pub vibe_home: PathBuf,
    /// `HOME` override (test isolation); `None` keeps the user's.
    pub home: Option<PathBuf>,
    /// Working directory of the server process.
    pub cwd: PathBuf,
    /// Explicit variables from our own code (validated against the hijack list).
    pub extra_env: Vec<(String, OsString)>,
    /// Parent environment to filter; `None` reads this process's environment at each start.
    pub parent_env: Option<Vec<(OsString, OsString)>>,
    /// Our `initialize` params.
    pub initialize: InitializeParams,
    /// Transport tuning (the key is added to its stderr redactions at each start).
    pub connection: ConnectionConfig,
    /// Bound for the `initialize` answer.
    pub handshake_timeout: Duration,
    /// Restart policy.
    pub backoff: Backoff,
    /// After closing stdin, how long the server may take to exit before it is killed.
    pub shutdown_grace: Duration,
    /// Unix crash containment (`child.rs`): when THIS process dies without stopping the server, a
    /// watchdog waits this long for the server's graceful exit on stdin EOF, then kills the server's
    /// process group. `None` disables it (the next start's ledger sweep remains). Default 5 s. Windows
    /// ignores it: the kill-on-close job ends the tree at once.
    pub watchdog_grace: Option<Duration>,
    /// Bound for `session/resume` (a reopen right after a restart can take tens of seconds; allow
    /// enough time for recovery).
    pub resume_timeout: Duration,
    /// The resume wait we expect after a crash, shown with the progress.
    pub resume_expected: Duration,
    /// How often resume progress is published.
    pub progress_interval: Duration,
    /// Server-request handlers, registered on every new connection (restarts included).
    pub handlers: Vec<(String, Arc<dyn ServerRequestHandler>)>,
    /// Core-owned filesystem checks, completed before every first, deferred or restarted server spawn.
    pub before_launch: Option<Arc<LaunchCheck>>,
}

impl SupervisorConfig {
    /// A config with production defaults for the given program, home and working directory.
    #[must_use]
    pub fn new(program: PathBuf, vibe_home: PathBuf, cwd: PathBuf) -> Self {
        Self {
            program,
            launcher: Vec::new(),
            extra_args: Vec::new(),
            mode: ServerMode::Multiplex,
            vibe_home,
            home: None,
            cwd,
            extra_env: Vec::new(),
            parent_env: None,
            initialize: default_initialize_params(),
            connection: ConnectionConfig::default(),
            handshake_timeout: Duration::from_secs(30),
            backoff: Backoff::default(),
            shutdown_grace: Duration::from_secs(5),
            watchdog_grace: Some(Duration::from_secs(5)),
            resume_timeout: Duration::from_secs(150),
            resume_expected: Duration::from_secs(31),
            progress_interval: Duration::from_secs(1),
            handlers: Vec::new(),
            before_launch: None,
        }
    }
}

/// A session whose `session/resume` is in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovering {
    /// The session being resumed.
    pub session_id: String,
    /// Time since the resume was sent.
    pub elapsed: Duration,
    /// Expected wait, shown with the progress.
    pub expected: Duration,
}

/// What the supervisor is doing; observe it through `Supervisor::state`.
#[derive(Debug, Clone, PartialEq)]
pub enum SupervisorState {
    /// First start in progress.
    Starting,
    /// No server to start yet: Vibe is not installed. The user installs it at setup; the
    /// first server starts with [`crate::Supervisor::launch_installed`]. Nothing is spawned in this state.
    NotInstalled {
        /// Why (shown to the user), e.g. "Vibe is not installed yet".
        reason: String,
    },
    /// A server is running and initialized.
    Ready {
        /// Starts so far (1 for the first server; +1 per restart).
        generation: u64,
        /// `initialize` answer of this server (name and version).
        server: ServerInfo,
        /// Tracked sessions that existed before this server started and still need
        /// `session/resume` on it (empty after the first start).
        needs_resume: Vec<String>,
        /// Sessions whose resume is in progress, with elapsed time.
        recovering: Vec<Recovering>,
    },
    /// The server died; a restart is scheduled.
    Restarting {
        /// Restart attempt number since the last stable run (1-based).
        attempt: u32,
        /// Delay before this attempt.
        retry_in: Duration,
        /// How the previous server died.
        last_death: DeathReport,
    },
    /// Shut down on request.
    Stopped {
        /// Exit status of the last server, if known.
        exit: Option<ExitInfo>,
    },
    /// Gave up after too many restarts.
    Failed {
        /// Restarts tried.
        attempts: u32,
        /// How the last server died, if one ran.
        last_death: Option<DeathReport>,
        /// Last start error (no server output in it).
        reason: String,
    },
}

/// Supervisor failure.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    /// A core-owned prelaunch check refused to start the server.
    #[error("server startup check failed: {0}")]
    BeforeLaunch(String),
    /// The requested feature is not available in this mode (e.g. channels on a single-connection
    /// server).
    #[error("not supported: {0}")]
    Unsupported(&'static str),
    /// The secret provider failed.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// An explicit environment variable was refused.
    #[error(transparent)]
    Env(#[from] EnvError),
    /// The app-owned startup file for agent command shells could not be prepared.
    #[error("preparing agent shell startup {} failed: {source}", .path.display())]
    ShellStartup {
        /// The managed startup file.
        path: PathBuf,
        /// The filesystem error.
        source: std::io::Error,
    },
    /// The configured program is relative or not an executable file. The supervisor only spawns
    /// an absolute, canonical path (never a bare name: a relative program may resolve against
    /// the child's working directory, where an agent could plant a binary).
    #[error("the server program must be an absolute path to an executable file: {}", .0.display())]
    ProgramNotUsable(PathBuf),
    /// The process could not be started.
    #[error("starting {} failed: {source}", .program.display())]
    Spawn {
        /// argv[0] that failed.
        program: PathBuf,
        /// OS error.
        source: std::io::Error,
    },
    /// The process started but the `initialize` handshake failed. `stderr_tail` is sensitive
    /// server output and is not part of this error's `Display`.
    #[error("initialize handshake failed: {source}")]
    Handshake {
        /// The transport error (boxed: it is large and errors travel by value).
        source: Box<RpcError>,
        /// Exit status, if the process exited.
        exit: Option<ExitInfo>,
        /// Last stderr lines (key redacted).
        stderr_tail: Vec<String>,
    },
    /// No ready server within the wait (state shows why).
    #[error("no ready server: {0}")]
    NotReady(String),
    /// A request on the current connection failed.
    #[error(transparent)]
    Rpc(#[from] RpcError),
}
