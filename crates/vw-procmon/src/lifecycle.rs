//! Lifecycle of what sessions run: what Vibe Work starts (`owned`) ends with its
//! session, at quit and, after a crash, at the next start; what the agent starts (`agent`) keeps running until
//! someone asks to stop it, stays tied to its session through the ledger, and is re-attached after a restart.
//!
//! Depends on: `monitor` (`ProcMon` internals), `ledger`, `stop`, `attribution`, `os`, `view`. Called by:
//! vw-core-api (start: sweep; delete/archive: `end_session`; quit: `shutdown`; tick: `refresh_ledger`),
//! tests. What a deleted session leaves running is `left_running`.
//! Invariants:
//! - Automatic stops (session end, quit, sweep) only ever touch `owned` processes with a proven identity
//!   (pid + start token); `agent` processes are only listed.
//! - The sweep never touches the ledger of a live instance, nor an entry whose pid now belongs to another
//!   process, nor one marked keep-running.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::attribution::{Adopted, Attribution, SessionContext};
use crate::error::LedgerError;
use crate::ledger::{Ledger, LedgerEntry};
use crate::monitor::ProcMon;
use crate::os;
use crate::registry::{ProcessClass, ProcessGroup};
use crate::stop::{Authority, StopOutcome, stop};
use crate::types::{ProcessInfo, ProcessRef, Snapshot};
use crate::view::{ProcessView, SessionEndReport};

/// The running `vibe-app-server`s are Vibe Work's own (block `contain`): recorded as `owned`, app-wide, with
/// their process group (Unix) / job (Windows), so a server that outlives a crash and its watchdog is stopped
/// by the next start's sweep.
fn server_entries(
    snapshot: &Snapshot,
    servers: &[ProcessRef],
    seen: &mut BTreeSet<ProcessRef>,
) -> Vec<LedgerEntry> {
    let mut entries = Vec::new();
    for server in servers {
        let Some(info) = snapshot
            .alive(server.pid)
            .filter(|p| p.start == Some(server.start))
        else {
            continue;
        };
        seen.insert(*server);
        entries.push(LedgerEntry {
            process: *server,
            group: if cfg!(windows) {
                ProcessGroup::WindowsJob {
                    name: "vibe-app-server".to_owned(),
                }
            } else {
                info.pgid
                    .map_or(ProcessGroup::None, |pgid| ProcessGroup::Unix { pgid })
            },
            session_id: None,
            class: ProcessClass::Owned,
            name: info.exe_name(),
            arg_count: words_of(info).len().saturating_sub(1),
            keep_running: false,
            ports: snapshot.ports_of(server.pid),
        });
    }
    entries
}

/// One ledger entry and what the sweep did with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepItem {
    /// The entry.
    pub entry: LedgerEntry,
    /// The stop, when one ran.
    pub outcome: Option<StopOutcome>,
    /// Why it failed or was refused, when it did.
    pub error: Option<String>,
}

/// What the startup sweep found and did ("Stopped 2 processes left from last time").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    /// Leftover `owned` processes stopped.
    pub stopped: Vec<SweepItem>,
    /// `owned` processes that could not be stopped.
    pub failed: Vec<SweepItem>,
    /// `agent` processes still running, re-attached to their session.
    pub reattached: Vec<SweepItem>,
    /// `agent` processes whose session no longer exists: listed for the user (Stop offered), not stopped.
    pub deleted_session: Vec<SweepItem>,
    /// Entries whose pid now belongs to another process: not touched.
    pub pid_reused: Vec<SweepItem>,
    /// Entries marked "keep running after Vibe Work closes": not touched.
    pub kept_running: Vec<SweepItem>,
    /// Entries whose process is gone.
    pub gone: usize,
    /// Ledgers of other instances that are still running: not touched.
    pub live_instances: Vec<ProcessRef>,
    /// Ledger files that could not be read (left in place).
    pub unreadable: Vec<String>,
}

/// The command line of a scanned process (Linux reads it lazily, so it may still be empty in the scan).
fn words_of(info: &crate::types::ProcessInfo) -> Vec<String> {
    if info.cmdline.is_empty() {
        os::cmdline(info.pid)
    } else {
        info.cmdline.clone()
    }
}

fn item(entry: &LedgerEntry) -> SweepItem {
    SweepItem {
        entry: entry.clone(),
        outcome: None,
        error: None,
    }
}

impl ProcMon {
    /// Startup sweep over the ledgers of earlier instances. `exists(session)` says whether a session still
    /// exists (`None` = unknown: treated as existing, so nothing is listed as orphaned on a guess).
    /// Blocking.
    ///
    /// # Errors
    /// [`LedgerError`] when no ledger is configured-and-readable at all; single bad files are reported in
    /// the result instead.
    pub fn sweep(&self, exists: &dyn Fn(&str) -> Option<bool>) -> Result<SweepReport, LedgerError> {
        let mut report = SweepReport::default();
        let Some(ledger) = self.inner.ledger.as_ref() else {
            return Ok(report);
        };
        let (others, corrupt) = ledger.others()?;
        report.unreadable = corrupt.iter().map(ToString::to_string).collect();
        let snapshot = os::snapshot()?;
        let mut carried: Vec<LedgerEntry> = Vec::new();
        for other in others {
            if os::is_alive(other.file.instance) {
                report.live_instances.push(other.file.instance);
                continue;
            }
            for entry in &other.file.entries {
                self.sweep_entry(entry, &snapshot, exists, &mut report, &mut carried);
            }
            if let Err(error) = Ledger::retire(&other.path) {
                report.unreadable.push(error.to_string());
            }
        }
        let mut entries = ledger.entries();
        entries.extend(carried);
        ledger.replace(entries)?;
        Ok(report)
    }

    fn sweep_entry(
        &self,
        entry: &LedgerEntry,
        snapshot: &Snapshot,
        exists: &dyn Fn(&str) -> Option<bool>,
        report: &mut SweepReport,
        carried: &mut Vec<LedgerEntry>,
    ) {
        let current = snapshot.alive(entry.process.pid);
        match current {
            None => {
                report.gone += 1;
                return;
            }
            Some(p) if p.start != Some(entry.process.start) => {
                report.pid_reused.push(item(entry));
                return;
            }
            Some(_) => {}
        }
        if entry.keep_running {
            report.kept_running.push(item(entry));
            carried.push(entry.clone());
            return;
        }
        match entry.class {
            ProcessClass::Owned => {
                let inputs = self.inputs(&[]);
                let mut done = item(entry);
                match stop(
                    entry.process,
                    &Authority::Ledger,
                    &inputs,
                    &self.inner.config.stop,
                ) {
                    Ok(outcome) if outcome.gone => {
                        done.outcome = Some(outcome);
                        report.stopped.push(done);
                    }
                    Ok(outcome) => {
                        done.error = Some(format!("still running: {:?}", outcome.remaining));
                        done.outcome = Some(outcome);
                        report.failed.push(done);
                        carried.push(entry.clone());
                    }
                    Err(error) => {
                        done.error = Some(error.to_string());
                        report.failed.push(done);
                        carried.push(entry.clone());
                    }
                }
            }
            ProcessClass::Agent => {
                let session = entry.session_id.clone().unwrap_or_default();
                if exists(&session) == Some(false) {
                    report.deleted_session.push(item(entry));
                    self.note_left_running(entry.clone());
                } else {
                    report.reattached.push(item(entry));
                    self.lock_adopted().push(Adopted {
                        session_id: session.clone(),
                        process: entry.process,
                        class: ProcessClass::Agent,
                    });
                    for port in &entry.ports {
                        let _ = self.inner.leases.adopt(&session, *port);
                    }
                }
                carried.push(entry.clone());
            }
        }
    }

    fn lock_adopted(&self) -> std::sync::MutexGuard<'_, Vec<Adopted>> {
        self.inner
            .adopted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_deleted(&self) -> std::sync::MutexGuard<'_, Vec<LedgerEntry>> {
        self.inner
            .deleted_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Rescans and rewrites the ledger: registered processes, the running `vibe-app-server`s (owned,
    /// app-wide), re-attached ones still alive, and every top
    /// `agent` process attributed to a session (a process whose parent is not itself an agent process of the
    /// same session). Blocking.
    ///
    /// # Errors
    /// [`LedgerError`] when the scan or the write fails.
    pub fn refresh_ledger(
        &self,
        sessions: &[SessionContext],
    ) -> Result<Vec<LedgerEntry>, LedgerError> {
        let snapshot = self.scan()?;
        let inputs = self.inputs(sessions);
        let mut attribution = Attribution::new(&snapshot, &inputs);
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        for own in &inputs.own {
            if snapshot
                .alive(own.process.pid)
                .is_some_and(|p| p.start == Some(own.process.start))
            {
                let words = snapshot
                    .processes
                    .get(&own.process.pid)
                    .map_or_else(|| os::cmdline(own.process.pid), words_of);
                seen.insert(own.process);
                entries.push(LedgerEntry {
                    process: own.process,
                    group: own.group.clone(),
                    session_id: own.session_id.clone(),
                    class: own.class,
                    name: snapshot
                        .processes
                        .get(&own.process.pid)
                        .and_then(ProcessInfo::exe_name),
                    arg_count: words.len().saturating_sub(1),
                    keep_running: own.keep_running,
                    ports: snapshot.ports_of(own.process.pid),
                });
            }
        }
        entries.extend(server_entries(&snapshot, &inputs.vibe_servers, &mut seen));
        for info in snapshot
            .processes
            .values()
            .filter(|p| p.alive && p.same_user == Some(true))
        {
            let Some(reference) = info.reference() else {
                continue;
            };
            if seen.contains(&reference) {
                continue;
            }
            let result = attribution.process(info.pid);
            let (Some(session), Some(ProcessClass::Agent)) =
                (result.owner.session(), result.provenance)
            else {
                continue;
            };
            let parent_same = snapshot.parent_of(info).is_some_and(|parent| {
                let up = attribution.process(parent.pid);
                up.owner.session() == Some(session) && up.provenance == Some(ProcessClass::Agent)
            });
            if parent_same {
                continue;
            }
            entries.push(LedgerEntry {
                process: reference,
                group: info
                    .pgid
                    .map_or(ProcessGroup::None, |pgid| ProcessGroup::Unix { pgid }),
                session_id: Some(session.to_owned()),
                class: ProcessClass::Agent,
                name: info.exe_name(),
                arg_count: words_of(info).len().saturating_sub(1),
                keep_running: false,
                ports: snapshot.ports_of(info.pid),
            });
        }
        // Re-attached processes stay remembered while they live, also when no session context names them.
        let adopted = self.lock_adopted().clone();
        let previous = self
            .inner
            .ledger
            .as_ref()
            .map(Ledger::entries)
            .unwrap_or_default();
        for adopted in adopted {
            let listed = entries.iter().any(|e| e.process == adopted.process);
            if !listed
                && snapshot
                    .alive(adopted.process.pid)
                    .is_some_and(|p| p.start == Some(adopted.process.start))
                && let Some(old) = previous.iter().find(|e| e.process == adopted.process)
            {
                entries.push(old.clone());
            }
        }
        for deleted in self.lock_deleted().iter() {
            if !entries.iter().any(|e| e.process == deleted.process)
                && os::is_alive(deleted.process)
            {
                entries.push(deleted.clone());
            }
        }
        if let Some(ledger) = &self.inner.ledger {
            ledger.replace(entries.clone())?;
        }
        Ok(entries)
    }

    /// Ends a session (archive, delete): stops its `owned` processes, lists its `agent` processes (left
    /// running; they stay tied to the session through the ledger), checks that the stopped processes' ports
    /// are free, and returns unused lease slots. Pending/live agent starts keep their assignments until
    /// they exit. Blocking.
    #[must_use]
    pub fn end_session(&self, session_id: &str, sessions: &[SessionContext]) -> SessionEndReport {
        let mut report = SessionEndReport {
            session_id: session_id.to_owned(),
            stopped: Vec::new(),
            failed: Vec::new(),
            agent_running: Vec::new(),
            leftover_ports: Vec::new(),
            released_ports: Vec::new(),
        };
        let inputs = self.inputs(sessions);
        for own in self.inner.registry.for_session(session_id) {
            if own.class != ProcessClass::Owned || own.keep_running {
                continue;
            }
            match stop(
                own.process,
                &Authority::Session(session_id.to_owned()),
                &inputs,
                &self.inner.config.stop,
            ) {
                Ok(outcome) => {
                    self.inner.registry.unregister_process(own.process);
                    report.stopped.push(outcome);
                }
                Err(error) => report
                    .failed
                    .push(format!("{} ({}): {error}", own.label, own.process.pid)),
            }
        }
        if let Ok(snapshot) = self.scan() {
            let mut attribution = Attribution::new(&snapshot, &inputs);
            for info in snapshot.processes.values().filter(|p| p.alive) {
                let result = attribution.process(info.pid);
                if result.owner.session() == Some(session_id)
                    && result.provenance == Some(ProcessClass::Agent)
                {
                    let parent_same = snapshot.parent_of(info).is_some_and(|parent| {
                        let up = attribution.process(parent.pid);
                        up.owner.session() == Some(session_id)
                            && up.provenance == Some(ProcessClass::Agent)
                    });
                    if !parent_same {
                        // Full view only when proven; Vibe's own processes are inferred: name + pid.
                        let mut view = ProcessView::for_attributed(info, &snapshot, &result);
                        // Vibe started it (provenance proven): its ports help the user decide.
                        view.class = Some(ProcessClass::Agent);
                        view.ports = snapshot.ports_of(info.pid);
                        if let Some(process) = info.reference() {
                            view.ports.extend(self.inner.leases.ports_of(process));
                            view.ports.sort_unstable();
                            view.ports.dedup();
                        }
                        report.agent_running.push(view);
                        if let Some(process) = info.reference() {
                            let mut adopted = self.lock_adopted();
                            if !adopted.iter().any(|a| a.process == process) {
                                adopted.push(Adopted {
                                    session_id: session_id.to_owned(),
                                    process,
                                    class: ProcessClass::Agent,
                                });
                            }
                        }
                    }
                }
            }
            let stopped_ports: BTreeSet<u16> = report
                .stopped
                .iter()
                .flat_map(|o| o.ports.iter().copied())
                .collect();
            report.leftover_ports = stopped_ports
                .into_iter()
                .filter(|port| snapshot.listeners.iter().any(|l| l.port == *port))
                .collect();
        }
        report.released_ports = self.inner.leases.release(session_id);
        let _ = self.refresh_ledger(sessions);
        report
    }

    /// Quit: stops every session's `owned` processes (agent processes keep running and stay in the ledger
    /// for the next start). Returns one report per session that had registered processes. Blocking.
    #[must_use]
    pub fn shutdown(&self, sessions: &[SessionContext]) -> Vec<SessionEndReport> {
        let mut ids: BTreeSet<String> = self
            .inner
            .registry
            .list()
            .into_iter()
            .filter_map(|p| p.session_id)
            .collect();
        ids.extend(self.inner.leases.all().into_keys());
        ids.into_iter()
            .map(|id| self.end_session(&id, sessions))
            .collect()
    }
}
