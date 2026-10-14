//! The core's side of the process and port monitor: session contexts from the store,
//! the startup sweep, the slow tick, and the `Core` methods the Debug API (read only), the MCP tools and
//! the UI use. The monitor itself is `vw-procmon`.
//!
//! Depends on: vw-procmon, vw-session (store views), `core::Shared`, `error`. Called by: `core` (start,
//! shutdown), `session_end` (archive, delete), `left_running`, vw-debug-api.
//! Invariants:
//! - Every OS scan runs on `spawn_blocking`, never on the async runtime.
//! - Session end (delete/archive) and quit stop only `owned` processes; `agent` processes are listed
//!   with their session; those of a deleted session are offered to the user (`left_running.rs`). The
//!   vibe-app-server itself is stopped by the supervisor, not here.
//! - The tick runs only while sessions are loaded, and is owned and aborted by the core.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio::task::JoinHandle;
use vw_procmon::{
    BackgroundProcess, PortExplanation, ProcMon, ProcessRef, SessionContext, SessionProcesses,
    StopError, StopOutcome, SweepReport,
};
use vw_session::SessionStore;

use crate::core::{Core, Shared};
use crate::error::{CoreError, ErrorCode};

/// Session contexts (folders, Vibe background processes) of every loaded session, from the store.
pub(crate) fn contexts(store: &SessionStore) -> Vec<SessionContext> {
    store
        .views()
        .into_iter()
        .map(|view| {
            let state = &view.state;
            let mut folders: Vec<PathBuf> = Vec::new();
            if let Some(cwd) = state.pointer("/session/cwd").and_then(Value::as_str) {
                folders.push(PathBuf::from(cwd));
            }
            if let Some(roots) = state
                .pointer("/session/workspaceRoots")
                .and_then(Value::as_array)
            {
                folders.extend(roots.iter().filter_map(Value::as_str).map(PathBuf::from));
            }
            let background = state
                .pointer("/backgroundProcesses")
                .and_then(Value::as_array)
                .map(|all| {
                    all.iter()
                        .map(|p| BackgroundProcess {
                            process_id: text(p, "processId"),
                            command: text(p, "command"),
                            status: text(p, "status"),
                            output: String::new(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            SessionContext {
                session_id: view.session_id,
                folders,
                background,
            }
        })
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Tells the monitor which `vibe-app-server` runs now (its descendants are the agent's processes) and
/// remembers its pid for the tick (a new server must reach the ledger even without loaded sessions).
pub(crate) fn note_vibe_server(shared: &Shared) {
    let pid = shared.supervisor.server_pid();
    let server = pid.and_then(vw_procmon::identity);
    shared
        .procmon
        .set_vibe_servers(server.into_iter().collect());
    shared
        .ledger_server_pid
        .store(pid.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

/// Records the running server in the ledger now (block `contain`: the startup sweep after a crash stops a
/// server that outlived it). No-op without a ledger.
pub(crate) async fn record_server(shared: &Arc<Shared>) {
    if shared.procmon.ledger().is_none() {
        return;
    }
    note_vibe_server(shared);
    let monitor = shared.procmon.clone();
    let sessions = contexts(&shared.store);
    match tokio::task::spawn_blocking(move || monitor.refresh_ledger(&sessions)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "ledger: server not recorded"),
        Err(error) => tracing::warn!(%error, "ledger task failed"),
    }
}

/// Aborts a task when dropped (the monitor tick lives and dies with the task that owns it).
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts the slow tick: the monitor's ledger refresh (while sessions are loaded, or when the server
/// changed) and, on the same period, one pass of the background-process output reader. Aborting the
/// returned task stops both.
pub(crate) fn spawn_tick(shared: &Arc<Shared>) -> JoinHandle<()> {
    let weak_active = Arc::downgrade(shared);
    let weak_contexts = Arc::downgrade(shared);
    let ledger = shared.procmon.spawn_tick(
        move || {
            weak_active.upgrade().is_some_and(|s| {
                let recorded = s
                    .ledger_server_pid
                    .load(std::sync::atomic::Ordering::Relaxed);
                !s.store.statuses().is_empty()
                    || (s.procmon.ledger().is_some()
                        && s.supervisor.server_pid().unwrap_or(0) != recorded)
            })
        },
        move || {
            weak_contexts.upgrade().map_or_else(Vec::new, |s| {
                note_vibe_server(&s);
                contexts(&s.store)
            })
        },
    );
    let weak = Arc::downgrade(shared);
    let period = shared
        .config
        .procmon
        .tick
        .max(std::time::Duration::from_millis(100));
    tokio::spawn(async move {
        let _ledger = AbortOnDrop(ledger);
        let mut reader = crate::background_output::OutputReader::default();
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            reader.pass(&shared).await;
        }
    })
}

/// Runs a blocking monitor call off the async runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work).await.map_err(|e| {
        CoreError::new(
            ErrorCode::Internal,
            format!("process monitor task failed: {e}"),
        )
    })
}

fn scan_error(error: &impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorCode::Internal, format!("process monitor: {error}"))
}

/// Ends a session in the monitor (delete or archive): stops its `owned` processes, keeps its `agent` ones
/// running and listed, returns its lease. On delete its agent processes move to the "left running" list (
/// `session_end.rs` asks about them). Returns the monitor's report (`None` when the monitor task failed).
pub(crate) async fn end_session(
    shared: &Shared,
    session_id: &str,
    deleted: bool,
) -> Option<vw_procmon::SessionEndReport> {
    let monitor = shared.procmon.clone();
    let sessions = contexts(&shared.store);
    let id = session_id.to_owned();
    blocking(move || {
        let report = monitor.end_session(&id, &sessions);
        monitor.forget_outputs(&id);
        if deleted {
            monitor.session_deleted(&id, &report.agent_running);
        }
        report
    })
    .await
    .ok()
}

impl Core {
    /// The process and port monitor (registry for terminals, leases, ledger).
    #[must_use]
    pub fn procmon(&self) -> &ProcMon {
        &self.shared.procmon
    }

    /// Session contexts of the loaded sessions (what attribution reads).
    #[must_use]
    pub fn session_contexts(&self) -> Vec<SessionContext> {
        contexts(&self.shared.store)
    }

    /// A loaded session's processes, listeners and leases, from a fresh scan (Debug API
    /// `GET /sessions/{id}/processes`).
    ///
    /// # Errors
    /// `notFound` when the session is not loaded; `internal` when the OS tables cannot be read.
    pub async fn session_processes(&self, session_id: &str) -> Result<SessionProcesses, CoreError> {
        if self.shared.store.view(session_id).is_none() {
            return Err(CoreError::not_loaded(session_id));
        }
        note_vibe_server(&self.shared);
        let monitor = self.shared.procmon.clone();
        let sessions = self.session_contexts();
        let id = session_id.to_owned();
        blocking(move || monitor.session_view(&id, &sessions))
            .await?
            .map_err(|e| scan_error(&e))
    }

    /// Leases ports to a loaded session (tops its lease up to the configured size).
    ///
    /// # Errors
    /// `notFound` when the session is not loaded; `internal` when the range is exhausted or the scan fails.
    pub async fn lease_ports(&self, session_id: &str) -> Result<Vec<u16>, CoreError> {
        if self.shared.store.view(session_id).is_none() {
            return Err(CoreError::not_loaded(session_id));
        }
        let monitor = self.shared.procmon.clone();
        let id = session_id.to_owned();
        blocking(move || monitor.lease(&id))
            .await?
            .map_err(|e| scan_error(&e))
    }

    /// Explains who holds `port` and what `asking` can do about it for the UI.
    ///
    /// # Errors
    /// `internal` when the OS tables cannot be read.
    pub async fn explain_port(
        &self,
        asking: Option<String>,
        port: u16,
    ) -> Result<PortExplanation, CoreError> {
        note_vibe_server(&self.shared);
        let monitor = self.shared.procmon.clone();
        let sessions = self.session_contexts();
        blocking(move || monitor.explain_port(asking.as_deref(), port, &sessions))
            .await?
            .map_err(|e| scan_error(&e))
    }

    /// Stops a process on behalf of a session (its own agent via the tool). Refused unless a fresh scan
    /// proves it is that session's (facts tier; an owner inferred from the folder is refused).
    ///
    /// # Errors
    /// `denied` with the refusal reason (owner named); `internal` on OS failures.
    pub async fn stop_process(
        &self,
        session_id: &str,
        target: ProcessRef,
    ) -> Result<StopOutcome, CoreError> {
        note_vibe_server(&self.shared);
        let monitor = self.shared.procmon.clone();
        let sessions = self.session_contexts();
        let id = session_id.to_owned();
        blocking(move || monitor.stop(&id, target, &sessions))
            .await?
            .map_err(|e| match e {
                StopError::Refused(refusal) => {
                    CoreError::new(ErrorCode::Denied, refusal.to_string())
                }
                other => scan_error(&other),
            })
    }

    /// Stops a process the user confirmed in the UI after seeing its name, pid and owner: any process of
    /// one of our sessions (also an inferred one), never another app's.
    ///
    /// # Errors
    /// `denied` with the refusal reason (owner named); `internal` on OS failures.
    pub async fn stop_process_for_user(
        &self,
        target: ProcessRef,
    ) -> Result<StopOutcome, CoreError> {
        note_vibe_server(&self.shared);
        let monitor = self.shared.procmon.clone();
        let sessions = self.session_contexts();
        blocking(move || monitor.stop_confirmed_by_user(target, &sessions))
            .await?
            .map_err(|e| match e {
                StopError::Refused(refusal) => {
                    CoreError::new(ErrorCode::Denied, refusal.to_string())
                }
                other => scan_error(&other),
            })
    }

    /// The full command line of a process for the app window's "Show command" (trusted surface; never on the
    /// Debug API). Only for a process proven to be `session_id`'s.
    ///
    /// # Errors
    /// `denied` with the reason when the process is not proven to be the session's; `internal` on OS
    /// failures.
    pub async fn process_command_line(
        &self,
        session_id: &str,
        target: ProcessRef,
    ) -> Result<Vec<String>, CoreError> {
        note_vibe_server(&self.shared);
        let monitor = self.shared.procmon.clone();
        let sessions = self.session_contexts();
        let id = session_id.to_owned();
        blocking(move || monitor.command_line(&id, target, &sessions))
            .await?
            .map_err(|e| match e {
                StopError::Refused(refusal) => {
                    CoreError::new(ErrorCode::Denied, refusal.to_string())
                }
                other => scan_error(&other),
            })
    }

    /// What the startup sweep found and did ("Stopped N processes left from last time"; UI later).
    #[must_use]
    pub fn startup_sweep(&self) -> Option<SweepReport> {
        self.shared
            .sweep
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
