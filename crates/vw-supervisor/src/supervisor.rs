//! The supervisor: starts the server, hands out its connection (in multiplex mode: the lobby channel,
//! plus more channels on request), restarts it with backoff when it dies, tracks which sessions need
//! `session/resume`, reports resume progress, shuts down cleanly.
//!
//! Depends on: `launch` (spawn + connect + handshake), `child`, `config`, `backoff`; vw-rpc. Called by:
//! the app's core (vw-core-api: one supervisor per app), tests.
//! Invariants:
//! - One monitor task owns the running server; it is the only writer of `Restarting`/`Stopped`/
//!   `Failed`. `resume` only edits the `recovering` list inside `Ready`.
//! - A dead server is always killed (whole tree) and reaped before a new one starts.
//! - In multiplex mode the process dying ends every channel; the lobby channel dying on its own is
//!   treated as the server dying (kill, restart), because the core's event pump and listings depend on
//!   it and a lone failed channel means the process is in a state we do not understand.
//! - Dropping the supervisor aborts the monitor, which drops the child handle and kills the tree.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_rpc::{Connection, DeathReport, ExitInfo, Multiplexer, RequestOptions, TimeoutPolicy};

use crate::config::{Recovering, SupervisorConfig, SupervisorError, SupervisorState};
use crate::discovery::canonical_executable;
use crate::env::SecretProvider;
use crate::launch::{self, Link, Running};

/// A channel opened on the current multiplexed server, with the server generation it belongs to (a
/// restart makes every channel of the old generation dead).
pub struct OpenedChannel {
    /// The channel's connection (handshake done).
    pub conn: Connection,
    /// `SupervisorState::Ready::generation` of the server it lives on.
    pub generation: u64,
}

struct Inner {
    config: SupervisorConfig,
    /// The canonical executable that is spawned (set at start, or by `launch_installed` after a deferred start).
    program: Mutex<PathBuf>,
    secrets: Arc<dyn SecretProvider>,
    state: watch::Sender<SupervisorState>,
    conn: watch::Sender<Option<Connection>>,
    /// The multiplexer of the running server and its generation (multiplex mode only).
    mux: watch::Sender<Option<(Multiplexer, u64)>>,
    tracked: Mutex<BTreeSet<String>>,
    pid: Mutex<Option<u32>>,
}

/// Supervises one `vibe-app-server` process.
pub struct Supervisor {
    inner: Arc<Inner>,
    shutdown: watch::Sender<bool>,
    /// The monitor task; `None` until the first server runs (a deferred start has none yet).
    monitor: Mutex<Option<JoinHandle<()>>>,
    /// Serializes `launch_installed` (one first launch at a time).
    launching: tokio::sync::Mutex<()>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Some(monitor) = self
            .monitor
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            monitor.abort();
        }
    }
}

impl Supervisor {
    /// Starts the server, runs the handshake and begins supervising it.
    ///
    /// The first start is not retried: a failure here is a setup problem (wrong path, bad home)
    /// that the user should see at once.
    ///
    /// # Errors
    /// [`SupervisorError::ProgramNotUsable`] for a relative or non-executable program; secret, env,
    /// spawn and handshake errors.
    pub async fn start(
        mut config: SupervisorConfig,
        secrets: Arc<dyn SecretProvider>,
    ) -> Result<Self, SupervisorError> {
        if !config.program.is_absolute() {
            return Err(SupervisorError::ProgramNotUsable(config.program));
        }
        config.program = canonical_executable(&config.program)
            .ok_or_else(|| SupervisorError::ProgramNotUsable(config.program.clone()))?;
        let program = config.program.clone();
        let inner = Inner::new(config, program, secrets, SupervisorState::Starting);
        let running = inner.launch().await?;
        inner.publish_ready(&running, 1, Vec::new());
        let (shutdown, shutdown_rx) = watch::channel(false);
        let monitor = tokio::spawn(monitor(inner.clone(), running, shutdown_rx));
        Ok(Self {
            inner,
            shutdown,
            monitor: Mutex::new(Some(monitor)),
            launching: tokio::sync::Mutex::new(()),
        })
    }

    /// A supervisor with no server yet: Vibe is not installed. The state is
    /// [`SupervisorState::NotInstalled`] with `reason`; connections wait or fail like before a first start
    /// (`connection` returns `NotReady` after its wait). [`Supervisor::launch_installed`] starts the first
    /// server once Vibe is installed. Nothing is spawned here.
    #[must_use]
    pub fn start_pending(
        config: SupervisorConfig,
        secrets: Arc<dyn SecretProvider>,
        reason: String,
    ) -> Self {
        let program = config.program.clone();
        let inner = Inner::new(
            config,
            program,
            secrets,
            SupervisorState::NotInstalled { reason },
        );
        let (shutdown, _) = watch::channel(false);
        Self {
            inner,
            shutdown,
            monitor: Mutex::new(None),
            launching: tokio::sync::Mutex::new(()),
        }
    }

    /// Starts the first server of a [`Supervisor::start_pending`] supervisor from `program` (absolute; it is
    /// canonicalized and spawned exactly), runs the handshake, publishes `Ready` (generation 1) and begins
    /// supervising it like [`Supervisor::start`] does. On failure the state goes back to `NotInstalled` with
    /// the error as the reason, so the user can retry.
    ///
    /// # Errors
    /// [`SupervisorError::Unsupported`] when a server already runs (or the supervisor was not deferred, or
    /// was stopped); [`SupervisorError::ProgramNotUsable`]; secret, env, spawn and handshake errors.
    pub async fn launch_installed(&self, program: &std::path::Path) -> Result<(), SupervisorError> {
        let _one = self.launching.lock().await;
        if !matches!(
            &*self.inner.state.borrow(),
            SupervisorState::NotInstalled { .. }
        ) || *self.shutdown.borrow()
        {
            return Err(SupervisorError::Unsupported(
                "a server is already running or the supervisor stopped",
            ));
        }
        if !program.is_absolute() {
            return Err(SupervisorError::ProgramNotUsable(program.to_path_buf()));
        }
        let canonical = canonical_executable(program)
            .ok_or_else(|| SupervisorError::ProgramNotUsable(program.to_path_buf()))?;
        *self
            .inner
            .program
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = canonical;
        self.inner.state.send_replace(SupervisorState::Starting);
        match self.inner.launch().await {
            Ok(running) => {
                self.inner.publish_ready(&running, 1, Vec::new());
                let shutdown_rx = self.shutdown.subscribe();
                let monitor = tokio::spawn(monitor(self.inner.clone(), running, shutdown_rx));
                *self.monitor.lock().unwrap_or_else(PoisonError::into_inner) = Some(monitor);
                info!("first server started after the install");
                Ok(())
            }
            Err(error) => {
                self.inner
                    .state
                    .send_replace(SupervisorState::NotInstalled {
                        reason: format!("Vibe was installed but did not start: {error}"),
                    });
                Err(error)
            }
        }
    }

    /// The executable the supervisor spawns (canonical), or the configured one before a deferred launch.
    #[must_use]
    pub fn program(&self) -> PathBuf {
        self.inner
            .program
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The folder the server process runs in (the app's own `work/` folder in a real run): where a chat that is not
    /// about a user's project can open without touching one.
    #[must_use]
    pub fn work_dir(&self) -> PathBuf {
        self.inner.config.cwd.clone()
    }

    /// Observe the state (ready, restarting with attempt and delay, recovering sessions,...).
    #[must_use]
    pub fn state(&self) -> watch::Receiver<SupervisorState> {
        self.inner.state.subscribe()
    }

    /// The current connection, if a server is ready right now: the plain connection in single mode,
    /// the lobby channel in multiplex mode.
    #[must_use]
    pub fn current(&self) -> Option<Connection> {
        self.inner.conn.borrow().clone()
    }

    /// Waits up to `wait` for a ready server and returns its connection.
    ///
    /// # Errors
    /// [`SupervisorError::NotReady`] when the supervisor stopped, failed, or the wait expired.
    pub async fn connection(&self, wait: Duration) -> Result<Connection, SupervisorError> {
        let mut conn = self.inner.conn.subscribe();
        let mut state = self.inner.state.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(conn) = conn.borrow_and_update().clone() {
                return Ok(conn);
            }
            match &*state.borrow_and_update() {
                SupervisorState::Stopped { .. } => {
                    return Err(SupervisorError::NotReady("stopped".into()));
                }
                SupervisorState::Failed { reason, .. } => {
                    return Err(SupervisorError::NotReady(format!("failed: {reason}")));
                }
                SupervisorState::NotInstalled { reason } => {
                    return Err(SupervisorError::NotReady(reason.clone()));
                }
                _ => {}
            }
            tokio::select! {
                _ = conn.changed() => {}
                _ = state.changed() => {}
                () = tokio::time::sleep_until(deadline) => {
                    return Err(SupervisorError::NotReady(format!("no server within {wait:?}")));
                }
            }
        }
    }

    /// Opens one more channel on the current multiplexed server (waiting up to `wait` for one to be
    /// ready) and runs its `initialize` handshake. The server-request handlers of the config are
    /// installed on it before its first frame.
    ///
    /// # Errors
    /// [`SupervisorError::Unsupported`] in single mode; [`SupervisorError::NotReady`] when no server is
    /// ready in time; [`SupervisorError::Rpc`] when the process is dead or closing;
    /// [`SupervisorError::Handshake`] when the channel's handshake fails (the channel is closed again).
    pub async fn open_channel(&self, wait: Duration) -> Result<OpenedChannel, SupervisorError> {
        if self.inner.config.mode != crate::config::ServerMode::Multiplex {
            return Err(SupervisorError::Unsupported(
                "channels need ServerMode::Multiplex",
            ));
        }
        let (mux, generation) = self.multiplexer(wait).await?;
        let conn = launch::open_channel(&mux, &self.inner.config).await?;
        Ok(OpenedChannel { conn, generation })
    }

    /// Waits up to `wait` for a ready multiplexed server.
    async fn multiplexer(&self, wait: Duration) -> Result<(Multiplexer, u64), SupervisorError> {
        let mut mux = self.inner.mux.subscribe();
        let mut state = self.inner.state.subscribe();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(current) = mux.borrow_and_update().clone() {
                return Ok(current);
            }
            match &*state.borrow_and_update() {
                SupervisorState::Stopped { .. } => {
                    return Err(SupervisorError::NotReady("stopped".into()));
                }
                SupervisorState::Failed { reason, .. } => {
                    return Err(SupervisorError::NotReady(format!("failed: {reason}")));
                }
                SupervisorState::NotInstalled { reason } => {
                    return Err(SupervisorError::NotReady(reason.clone()));
                }
                _ => {}
            }
            tokio::select! {
                _ = mux.changed() => {}
                _ = state.changed() => {}
                () = tokio::time::sleep_until(deadline) => {
                    return Err(SupervisorError::NotReady(format!("no server within {wait:?}")));
                }
            }
        }
    }

    /// Records a session this client holds, so a restart lists it in `needs_resume`.
    pub fn track_session(&self, session_id: impl Into<String>) {
        self.inner.tracked().insert(session_id.into());
    }

    /// Forgets a session (closed by the user).
    pub fn untrack_session(&self, session_id: &str) {
        self.inner.tracked().remove(session_id);
    }

    /// OS process id of the current server leader (diagnostics and tests).
    #[must_use]
    pub fn server_pid(&self) -> Option<u32> {
        *self
            .inner
            .pid
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `session/resume` for `session_id` with the caller's params and publishes progress
    /// (`Ready.recovering`) every `progress_interval` until it answers. After a kill the unified
    /// harness can hold the session lock for ~31 s: callers see "recovering, 12 s of ~31 s"
    /// instead of a hang. On success the session leaves `needs_resume`.
    ///
    /// # Errors
    /// [`SupervisorError::NotReady`] when no server is ready within `resume_timeout`;
    /// [`SupervisorError::Rpc`] for the request's own failure (timeout after `resume_timeout`,
    /// error answer, server dead).
    pub async fn resume(&self, session_id: &str, params: Value) -> Result<Value, SupervisorError> {
        let conn = self.connection(self.inner.config.resume_timeout).await?;
        self.resume_on(&conn, session_id, params).await
    }

    /// [`Supervisor::resume`] on a given connection (in multiplex mode: the session's own channel).
    /// Same timeout and progress reporting.
    ///
    /// # Errors
    /// [`SupervisorError::Rpc`] for the request's own failure (timeout after `resume_timeout`, error
    /// answer, channel or server dead).
    pub async fn resume_on(
        &self,
        conn: &Connection,
        session_id: &str,
        params: Value,
    ) -> Result<Value, SupervisorError> {
        let config = &self.inner.config;
        let started = Instant::now();
        let options = RequestOptions {
            timeout: Some(config.resume_timeout),
            on_timeout: Some(TimeoutPolicy::Fail),
        };
        let request = conn.request_with("session/resume", params, options);
        tokio::pin!(request);
        let mut ticker = tokio::time::interval(config.progress_interval);
        let result = loop {
            tokio::select! {
                result = &mut request => break result,
                _ = ticker.tick() => self.inner.set_recovering(session_id, started.elapsed()),
            }
        };
        self.inner.clear_recovering(session_id, result.is_ok());
        info!(
            session_id,
            elapsed_ms = started.elapsed().as_millis(),
            ok = result.is_ok(),
            "session/resume finished"
        );
        Ok(result?)
    }

    /// [`Supervisor::shutdown`] through a shared handle (the core keeps the supervisor behind an
    /// `Arc`): asks the monitor to stop, then waits until the state is `Stopped` or `Failed`, at
    /// most `shutdown_grace` plus 10 s (the monitor's own kill and reap bounds). Returns the exit
    /// status if known. Calling it again, or after `Failed`, returns at once.
    pub async fn stop(&self) -> Option<ExitInfo> {
        // Held while a deferred first launch runs, so a stop never races `launch_installed`.
        let _one = self.launching.lock().await;
        self.shutdown.send_replace(true);
        let has_monitor = self
            .monitor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !has_monitor {
            self.inner
                .state
                .send_replace(SupervisorState::Stopped { exit: None });
            return None;
        }
        let mut state = self.inner.state.subscribe();
        let limit = self.inner.config.shutdown_grace + Duration::from_secs(10);
        let finished = tokio::time::timeout(
            limit,
            state.wait_for(|s| {
                matches!(
                    s,
                    SupervisorState::Stopped { .. } | SupervisorState::Failed { .. }
                )
            }),
        )
        .await
        .is_ok();
        if !finished {
            warn!(
                limit_ms = limit.as_millis(),
                "supervisor did not stop in time"
            );
        }
        match &*state.borrow() {
            SupervisorState::Stopped { exit } => *exit,
            _ => None,
        }
    }

    /// Clean shutdown: close the server's stdin, wait `shutdown_grace`, then kill the tree; reap
    /// everything. Returns the server's exit status if known.
    pub async fn shutdown(mut self) -> Option<ExitInfo> {
        self.shutdown.send_replace(true);
        let monitor = self
            .monitor
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match monitor {
            Some(monitor) => {
                let _ = monitor.await;
            }
            // Deferred and never launched: nothing runs, nothing to wait for.
            None => {
                self.inner
                    .state
                    .send_replace(SupervisorState::Stopped { exit: None });
            }
        }
        match &*self.inner.state.borrow() {
            SupervisorState::Stopped { exit } => *exit,
            _ => None,
        }
    }
}

impl Inner {
    fn tracked(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.tracked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn new(
        config: SupervisorConfig,
        program: PathBuf,
        secrets: Arc<dyn SecretProvider>,
        state: SupervisorState,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            program: Mutex::new(program),
            secrets,
            state: watch::Sender::new(state),
            conn: watch::Sender::new(None),
            mux: watch::Sender::new(None),
            tracked: Mutex::new(BTreeSet::new()),
            pid: Mutex::new(None),
        })
    }

    async fn launch(&self) -> Result<Running, SupervisorError> {
        let program = self
            .program
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        launch::launch(&self.config, &program, self.secrets.as_ref(), &self.pid).await
    }

    fn publish_ready(&self, running: &Running, generation: u64, needs_resume: Vec<String>) {
        self.conn.send_replace(Some(running.conn.clone()));
        self.mux
            .send_replace(running.mux().map(|mux| (mux.clone(), generation)));
        self.state.send_replace(SupervisorState::Ready {
            generation,
            server: running.server.clone(),
            needs_resume,
            recovering: Vec::new(),
        });
    }

    fn set_recovering(&self, session_id: &str, elapsed: Duration) {
        let expected = self.config.resume_expected;
        self.state.send_if_modified(|state| {
            let SupervisorState::Ready { recovering, .. } = state else {
                return false;
            };
            match recovering.iter_mut().find(|r| r.session_id == session_id) {
                Some(entry) => entry.elapsed = elapsed,
                None => recovering.push(Recovering {
                    session_id: session_id.to_owned(),
                    elapsed,
                    expected,
                }),
            }
            true
        });
    }

    fn clear_recovering(&self, session_id: &str, resumed: bool) {
        self.state.send_if_modified(|state| {
            let SupervisorState::Ready {
                recovering,
                needs_resume,
                ..
            } = state
            else {
                return false;
            };
            recovering.retain(|r| r.session_id != session_id);
            if resumed {
                needs_resume.retain(|id| id != session_id);
            }
            true
        });
    }
}

/// Owns the running server: waits for death or shutdown, restarts with backoff.
async fn monitor(inner: Arc<Inner>, mut running: Running, mut shutdown: watch::Receiver<bool>) {
    let backoff = inner.config.backoff;
    let mut generation = 1_u64;
    let mut attempt = 0_u32;
    loop {
        let death = tokio::select! {
            report = process_closed(&running) => Some(report),
            _ = shutdown.wait_for(|stop| *stop) => None,
        };
        let Some(mut report) = death else {
            inner.mux.send_replace(None);
            let exit = stop_gracefully(&inner, running).await;
            inner.conn.send_replace(None);
            inner.state.send_replace(SupervisorState::Stopped { exit });
            return;
        };
        inner.conn.send_replace(None);
        inner.mux.send_replace(None);
        let lived = running.started.elapsed();
        // Wedged or write-blocked servers are still alive: kill the tree, then reap.
        let exit = running.child.kill_and_reap(Duration::from_secs(5)).await;
        report.exit = report.exit.or(exit);
        warn!(
            cause = report.cause.kind(),
            exit_code = report.exit.and_then(|e| e.code),
            lived_ms = lived.as_millis(),
            "vibe-app-server died"
        );
        if lived >= backoff.stable_after {
            attempt = 0;
        }
        let mut last_error = String::from("server died");
        loop {
            attempt += 1;
            if attempt > backoff.max_attempts {
                inner.state.send_replace(SupervisorState::Failed {
                    attempts: attempt - 1,
                    last_death: Some(report),
                    reason: last_error,
                });
                return;
            }
            let retry_in = backoff.delay(attempt);
            inner.state.send_replace(SupervisorState::Restarting {
                attempt,
                retry_in,
                last_death: report.clone(),
            });
            tokio::select! {
                () = tokio::time::sleep(retry_in) => {}
                _ = shutdown.wait_for(|stop| *stop) => {
                    inner.state.send_replace(SupervisorState::Stopped { exit: report.exit });
                    return;
                }
            }
            match inner.launch().await {
                Ok(next) => {
                    running = next;
                    generation += 1;
                    let needs_resume: Vec<String> = inner.tracked().iter().cloned().collect();
                    inner.publish_ready(&running, generation, needs_resume);
                    break;
                }
                Err(error) => {
                    warn!(attempt, %error, "restart failed");
                    last_error = error.to_string();
                }
            }
        }
    }
}

/// Resolves when the server is gone: the plain connection died, or (multiplex) the process died or
/// its lobby channel died on its own (see the module invariants).
async fn process_closed(running: &Running) -> DeathReport {
    match &running.link {
        Link::Single => running.conn.closed().await,
        Link::Multiplex(mux) => tokio::select! {
            report = mux.closed() => report,
            report = running.conn.closed() => {
                warn!(cause = report.cause.kind(), "lobby channel died; treating the server as dead");
                report
            }
        },
    }
}

/// Closes stdin, waits the grace period, kills what is left and reaps it.
async fn stop_gracefully(inner: &Inner, running: Running) -> Option<ExitInfo> {
    match &running.link {
        Link::Single => running.conn.close_input().await,
        // Vibe closes every channel (each harness flushes and releases its lease) and exits.
        Link::Multiplex(mux) => mux.close_input().await,
    }
    let graceful = running.child.wait_exit(inner.config.shutdown_grace).await;
    if graceful.is_none() {
        warn!(
            grace_ms = inner.config.shutdown_grace.as_millis(),
            "server ignored stdin EOF; killing it"
        );
    }
    let forced = running.child.kill_and_reap(Duration::from_secs(5)).await;
    let _: Option<DeathReport> =
        tokio::time::timeout(Duration::from_secs(1), running.conn.closed())
            .await
            .ok();
    graceful.or(forced)
}
