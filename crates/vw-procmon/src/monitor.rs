//! `ProcMon`: the one owner of "what runs and who listens where". It holds the registry, the
//! leases, the output notes, the re-attached processes and the ledger; scans on demand and on a slow tick;
//! answers session views, busy-port explanations and stops. Session end, quit and the startup sweep live in
//! `lifecycle`.
//!
//! Depends on: `os`, `attribution`, `leases`, `registry`, `ledger`, `explain`, `stop`, `view`, tokio (tick).
//! Called by: vw-core-api (`Core` holds one), tests.
//! Invariants:
//! - Every OS scan is blocking; async callers run them on `spawn_blocking` (the core does).
//! - Session contexts come from the caller on every call (the core's store is the source of truth); the
//!   monitor only adds what it owns (output notes, canonical folder paths).
//! - Locks are plain mutexes, never held across an await or a scan.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::attribution::{Adopted, Attribution, Inputs, SessionContext, normalize};
use crate::error::{LeaseError, LedgerError, ScanError};
use crate::explain::{PortExplanation, explain};
use crate::leases::{LeaseConfig, PortLeases};
use crate::ledger::{Ledger, LedgerEntry};
use crate::os;
use crate::registry::OwnProcessRegistry;
use crate::stop::{Authority, StopConfig, StopError, StopOutcome, StopRefusal, stop};
use crate::text::busy_port_line;
use crate::types::{ProcessRef, Snapshot};
use crate::view::{BackgroundView, ListenerView, ProcessView, SessionProcesses};

/// Monitor settings. `Default` gives production values without a ledger (tests); the app sets
/// `ledger_dir` to a folder in its data directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcMonConfig {
    /// Lease range and size.
    pub leases: LeaseConfig,
    /// Stop timing.
    pub stop: StopConfig,
    /// Slow tick interval (only while something is active). Default 5 s.
    pub tick: Duration,
    /// Where the process ledger lives; `None` = no ledger.
    pub ledger_dir: Option<PathBuf>,
    /// Bytes of output kept per background process for URL corroboration. Default 16 KiB.
    pub output_tail_bytes: usize,
}

impl Default for ProcMonConfig {
    fn default() -> Self {
        Self {
            leases: LeaseConfig::default(),
            stop: StopConfig::default(),
            tick: Duration::from_secs(5),
            ledger_dir: None,
            output_tail_bytes: 16 * 1024,
        }
    }
}

pub(crate) struct Inner {
    pub(crate) config: ProcMonConfig,
    pub(crate) registry: OwnProcessRegistry,
    pub(crate) leases: PortLeases,
    pub(crate) outputs: Mutex<HashMap<(String, String), String>>,
    pub(crate) adopted: Mutex<Vec<Adopted>>,
    /// Agent processes whose session was deleted: listed for the user, never stopped automatically.
    pub(crate) deleted_session: Mutex<Vec<LedgerEntry>>,
    vibe_servers: Mutex<Vec<ProcessRef>>,
    last: Mutex<Option<Arc<Snapshot>>>,
    pub(crate) ledger: Option<Ledger>,
}

/// The monitor. Clone it freely (shared inside).
#[derive(Clone)]
pub struct ProcMon {
    pub(crate) inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ProcMon {
    /// A monitor. With `ledger_dir` set, opens this instance's ledger (named after this process's pid and
    /// start token; a scan reads the token).
    ///
    /// # Errors
    /// [`LedgerError`] when the ledger directory cannot be created or this process's identity read.
    pub fn new(config: ProcMonConfig) -> Result<Self, LedgerError> {
        let ledger = match &config.ledger_dir {
            Some(dir) => Some(Ledger::open(dir, self_identity()?)?),
            None => None,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                leases: PortLeases::new(config.leases.clone()),
                config,
                registry: OwnProcessRegistry::default(),
                outputs: Mutex::new(HashMap::new()),
                adopted: Mutex::new(Vec::new()),
                deleted_session: Mutex::new(Vec::new()),
                vibe_servers: Mutex::new(Vec::new()),
                last: Mutex::new(None),
                ledger,
            }),
        })
    }

    /// The registry of processes this app starts (the core registers each terminal shell here).
    #[must_use]
    pub fn registry(&self) -> &OwnProcessRegistry {
        &self.inner.registry
    }

    /// The lease table.
    #[must_use]
    pub fn leases(&self) -> &PortLeases {
        &self.inner.leases
    }

    /// The ledger, when configured.
    #[must_use]
    pub fn ledger(&self) -> Option<&Ledger> {
        self.inner.ledger.as_ref()
    }

    /// The running `vibe-app-server` processes (their descendants are the agent's processes).
    pub fn set_vibe_servers(&self, servers: Vec<ProcessRef>) {
        *lock(&self.inner.vibe_servers) = servers;
    }

    /// Records output text of a Vibe background process (kept as a bounded tail) for URL corroboration.
    /// The core's tick reads Vibe's `session/backgroundProcess/output` into it (`background_output`).
    pub fn note_output(&self, session_id: &str, process_id: &str, text: &str) {
        let mut outputs = lock(&self.inner.outputs);
        let tail = outputs
            .entry((session_id.to_owned(), process_id.to_owned()))
            .or_default();
        tail.push_str(text);
        let max = self.inner.config.output_tail_bytes;
        if tail.len() > max {
            let mut cut = tail.len() - max;
            while !tail.is_char_boundary(cut) {
                cut += 1;
            }
            tail.drain(..cut);
        }
    }

    /// Drops the output notes of a session that ended (archive, delete).
    pub fn forget_outputs(&self, session_id: &str) {
        lock(&self.inner.outputs).retain(|(session, _), _| session != session_id);
    }

    /// Builds attribution inputs: the caller's sessions (folders canonicalized, output notes added), the
    /// registry, the re-attached processes and the Vibe servers.
    #[must_use]
    pub fn inputs(&self, sessions: &[SessionContext]) -> Inputs {
        let outputs = lock(&self.inner.outputs).clone();
        let sessions = sessions
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.folders = s
                    .folders
                    .iter()
                    .map(|f| normalize(&f.canonicalize().unwrap_or_else(|_| f.clone())))
                    .collect();
                for bg in &mut s.background {
                    if let Some(text) = outputs.get(&(s.session_id.clone(), bg.process_id.clone()))
                    {
                        bg.output.push_str(text);
                    }
                }
                s
            })
            .collect();
        Inputs {
            sessions,
            own: self.inner.registry.list(),
            adopted: lock(&self.inner.adopted).clone(),
            vibe_servers: lock(&self.inner.vibe_servers).clone(),
        }
    }

    /// Scans the OS now and keeps the result as the latest snapshot. Blocking.
    ///
    /// # Errors
    /// [`ScanError`] when the OS tables cannot be read.
    pub fn scan(&self) -> Result<Arc<Snapshot>, ScanError> {
        self.inner.leases.reconcile_processes(os::is_alive);
        let snapshot = Arc::new(os::snapshot()?);
        *lock(&self.inner.last) = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// The latest snapshot (from a scan or the tick), if any.
    #[must_use]
    pub fn last_snapshot(&self) -> Option<Arc<Snapshot>> {
        lock(&self.inner.last).clone()
    }

    /// A session's listeners (owned and contested), registered processes, re-attached processes,
    /// background processes and leases, from a fresh scan. Blocking.
    ///
    /// # Errors
    /// [`ScanError`] when the OS tables cannot be read.
    pub fn session_view(
        &self,
        session_id: &str,
        sessions: &[SessionContext],
    ) -> Result<SessionProcesses, ScanError> {
        let mut snapshot = (*self.scan()?).clone();
        let inputs = self.inputs(sessions);
        let attributed = Attribution::new(&snapshot, &inputs).listeners();
        let pids: Vec<u32> = attributed.iter().filter_map(|l| l.listener.pid).collect();
        os::fill_cmdline(&mut snapshot, &pids);
        let mut listeners = Vec::new();
        let mut contested = Vec::new();
        for item in &attributed {
            match &item.attributed.owner {
                o if o.session() == Some(session_id) => {
                    listeners.push(ListenerView::new(item, &snapshot));
                }
                crate::attribution::Owner::OtherApp { candidates, .. }
                    if candidates.iter().any(|c| c == session_id) =>
                {
                    contested.push(ListenerView::new(item, &snapshot));
                }
                _ => {}
            }
        }
        let reattached = lock(&self.inner.adopted)
            .iter()
            .filter(|a| a.session_id == session_id)
            .filter_map(|a| snapshot.processes.get(&a.process.pid))
            .map(|info| {
                ProcessView::ours(info, &snapshot, Some(crate::registry::ProcessClass::Agent))
            })
            .collect();
        let background = sessions
            .iter()
            .filter(|s| s.session_id == session_id)
            .flat_map(|s| s.background.iter())
            .map(|b| BackgroundView {
                process_id: b.process_id.clone(),
                status: b.status.clone(),
            })
            .collect();
        Ok(SessionProcesses {
            session_id: session_id.to_owned(),
            listeners,
            contested,
            own: self.inner.registry.for_session(session_id),
            reattached,
            background,
            leases: self.inner.leases.of(session_id),
            scan_micros: u64::try_from(snapshot.took.as_micros()).unwrap_or(u64::MAX),
        })
    }

    /// Leases ports to a session (tops its lease up). Blocking (one scan + bind tests).
    ///
    /// # Errors
    /// [`LeaseError`] when the scan fails or the range is exhausted.
    pub fn lease(&self, session_id: &str) -> Result<Vec<u16>, LeaseError> {
        let snapshot = self.scan()?;
        let busy = |port: u16| snapshot.listeners.iter().any(|l| l.port == port);
        self.inner.leases.lease(session_id, &busy)
    }

    /// Explains a busy port for `asking`. Blocking.
    ///
    /// # Errors
    /// [`ScanError`] when the OS tables cannot be read.
    pub fn explain_port(
        &self,
        asking: Option<&str>,
        port: u16,
        sessions: &[SessionContext],
    ) -> Result<PortExplanation, ScanError> {
        let snapshot = self.scan()?;
        let inputs = self.inputs(sessions);
        Ok(explain(
            &snapshot,
            &inputs,
            &self.inner.leases,
            asking,
            Some(port),
        ))
    }

    /// Explains an "address already in use" output line; `Ok(None)` when the line is not one.
    ///
    /// # Errors
    /// [`ScanError`] when the OS tables cannot be read.
    pub fn explain_line(
        &self,
        asking: Option<&str>,
        line: &str,
        sessions: &[SessionContext],
    ) -> Result<Option<PortExplanation>, ScanError> {
        let Some(busy) = busy_port_line(line) else {
            return Ok(None);
        };
        let snapshot = self.scan()?;
        let inputs = self.inputs(sessions);
        Ok(Some(explain(
            &snapshot,
            &inputs,
            &self.inner.leases,
            asking,
            busy.port,
        )))
    }

    /// Stops `target` on behalf of `session_id` (its own agent through the tool). Refused unless a
    /// fresh scan attributes it to that session AS A FACT (registered, re-attached, or a descendant of
    /// those); an inferred owner is refused (`NotProven`). Blocking.
    ///
    /// # Errors
    /// [`StopError`] with the refusal reason, or on OS failures.
    pub fn stop(
        &self,
        session_id: &str,
        target: ProcessRef,
        sessions: &[SessionContext],
    ) -> Result<StopOutcome, StopError> {
        let inputs = self.inputs(sessions);
        let outcome = stop(
            target,
            &Authority::Session(session_id.to_owned()),
            &inputs,
            &self.inner.config.stop,
        )?;
        if outcome.gone {
            self.inner.registry.unregister_process(target);
            lock(&self.inner.adopted).retain(|a| a.process != target);
        }
        self.inner.leases.reconcile_processes(os::is_alive);
        Ok(outcome)
    }

    /// The full command line of `target`, for the app window's "Show command" (trusted surface, UI later).
    /// Only for a process proven (facts tier) to be `session_id`'s; never exposed through the Debug API,
    /// whose views carry no arguments. Blocking (one scan).
    ///
    /// # Errors
    /// The [`StopRefusal`] that a session stop of the same target would get (gone counts as `PidReused`
    /// when the pid now belongs to another process, else `InvalidPid`), or the scan error as `Scan`.
    pub fn command_line(
        &self,
        session_id: &str,
        target: ProcessRef,
        sessions: &[SessionContext],
    ) -> Result<Vec<String>, StopError> {
        let mut snapshot = (*self.scan()?).clone();
        let pid = target.pid;
        let Some(info) = snapshot.alive(pid) else {
            return Err(StopRefusal::InvalidPid {
                pid: u64::from(pid),
            }
            .into());
        };
        if info.start != Some(target.start) {
            return Err(StopRefusal::PidReused { pid }.into());
        }
        if info.same_user != Some(true) {
            return Err(StopRefusal::ForeignUser { pid }.into());
        }
        let inputs = self.inputs(sessions);
        let result = Attribution::new(&snapshot, &inputs).process(pid);
        crate::stop::authorize(pid, &result, &Authority::Session(session_id.to_owned()))?;
        os::fill_cmdline(&mut snapshot, &[pid]);
        Ok(snapshot
            .processes
            .get(&pid)
            .map(|p| p.cmdline.clone())
            .unwrap_or_default())
    }

    /// Stops `target` after the USER confirmed it in the UI (name, pid and owner shown): allowed for any
    /// process attributed to one of our sessions, also an inferred one; never another app's, another user's
    /// or this app. Blocking.
    ///
    /// # Errors
    /// [`StopError`] with the refusal reason, or on OS failures.
    pub fn stop_confirmed_by_user(
        &self,
        target: ProcessRef,
        sessions: &[SessionContext],
    ) -> Result<StopOutcome, StopError> {
        let inputs = self.inputs(sessions);
        let outcome = stop(target, &Authority::User, &inputs, &self.inner.config.stop)?;
        if outcome.gone {
            self.inner.registry.unregister_process(target);
            lock(&self.inner.adopted).retain(|a| a.process != target);
        }
        self.inner.leases.reconcile_processes(os::is_alive);
        Ok(outcome)
    }

    /// Starts the slow tick: every `config.tick`, while `active()` says so, rescans and refreshes the ledger
    /// with the sessions `contexts()` returns. The task is owned by the caller (abort it on shutdown).
    pub fn spawn_tick(
        &self,
        active: impl Fn() -> bool + Send + Sync + 'static,
        contexts: impl Fn() -> Vec<SessionContext> + Send + Sync + 'static,
    ) -> JoinHandle<()> {
        let monitor = self.clone();
        let period = self.inner.config.tick.max(Duration::from_millis(100));
        let contexts = Arc::new(contexts);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if !active() {
                    continue;
                }
                let monitor = monitor.clone();
                let contexts = contexts.clone();
                let done = tokio::task::spawn_blocking(move || monitor.refresh_ledger(&contexts()));
                if let Ok(Err(error)) = done.await {
                    tracing::warn!(%error, "process monitor tick failed");
                }
            }
        })
    }
}

/// This process's identity (pid + start token), for the ledger file name.
fn self_identity() -> Result<ProcessRef, LedgerError> {
    os::identity(std::process::id())
        .ok_or_else(|| ScanError::Unreadable("own start time".to_owned()).into())
}
