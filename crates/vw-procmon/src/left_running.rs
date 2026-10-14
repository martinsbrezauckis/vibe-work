//! What deleted sessions left running: the agent's processes (dev servers started with
//! `process_start`) keep running when their session is deleted, because only the user decides to stop them. They
//! are kept in one list, offered to the user ("Also stop its N running processes?" right after the delete, then a
//! "Left running" list with Stop), and never stopped automatically.
//!
//! Depends on: `monitor` (`ProcMon` internals), `ledger`, `stop`, `attribution`, `os`, `view`. Called by:
//! vw-core-api (delete: `session_deleted`; the `processes` command: `left_running`, `stop_left_running`), the
//! startup sweep (`lifecycle`, an earlier run's deleted sessions), tests.
//! Invariants:
//! - The list only holds identities this app recorded (pid + start token): from the ledger or from the end of the
//!   session that started them. A stop names one of them exactly; anything else is refused before a signal.
//! - A stop runs on the deleted session's authority with the recorded identity as a fact
//!   (`Authority::Session`): the process group may only hold processes proven to be that session's, so a group
//!   shared with the Vibe server, this app or another app is refused (`GroupShared`), never signalled.
//! - Reading the list drops processes that are gone (or whose pid now names another process), so it never grows
//!   past what is still running.

use crate::attribution::{Adopted, SessionContext};
use crate::ledger::{Ledger, LedgerEntry};
use crate::monitor::ProcMon;
use crate::os;
use crate::registry::{ProcessClass, ProcessGroup};
use crate::stop::{Authority, StopError, StopOutcome, StopRefusal, stop};
use crate::types::ProcessRef;
use crate::view::ProcessView;

impl ProcMon {
    fn lock_left(&self) -> std::sync::MutexGuard<'_, Vec<LedgerEntry>> {
        self.inner
            .deleted_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds an entry an earlier run's ledger names (startup sweep: its session no longer exists).
    pub(crate) fn note_left_running(&self, entry: LedgerEntry) {
        let mut left = self.lock_left();
        if !left.iter().any(|e| e.process == entry.process) {
            left.push(entry);
        }
    }

    /// Agent processes left running by deleted sessions that are still alive, oldest first. Gone ones are dropped
    /// from the list here. Blocking (one liveness check per entry).
    #[must_use]
    pub fn left_running(&self) -> Vec<LedgerEntry> {
        let mut left = self.lock_left();
        left.retain(|e| os::is_alive(e.process));
        left.clone()
    }

    /// A session was deleted: its agent processes (re-attached ones, and those `end_session` just listed in
    /// `still_running`) move to the "left running" list (still running, Stop offered to the user). Names and ports
    /// come from the ledger, else from the end report's views.
    pub fn session_deleted(&self, session_id: &str, still_running: &[ProcessView]) {
        let moved: Vec<Adopted> = {
            let mut adopted = self
                .inner
                .adopted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (gone, keep): (Vec<Adopted>, Vec<Adopted>) =
                adopted.drain(..).partition(|a| a.session_id == session_id);
            *adopted = keep;
            gone
        };
        let previous = self
            .inner
            .ledger
            .as_ref()
            .map(Ledger::entries)
            .unwrap_or_default();
        for a in moved {
            // The end report's view of the same scan: a process only inferred to be the session's is shown without
            // its start token (vw-procmon's view rule), so the pid matches it; a token, when shown, must match too.
            let seen = still_running.iter().find(|v| {
                v.pid == a.process.pid && v.start.is_none_or(|start| start == a.process.start)
            });
            let entry = previous
                .iter()
                .find(|e| e.process == a.process)
                .cloned()
                .unwrap_or_else(|| LedgerEntry {
                    process: a.process,
                    group: seen
                        .and_then(|v| v.pgid)
                        .map_or(ProcessGroup::None, |pgid| ProcessGroup::Unix { pgid }),
                    session_id: Some(session_id.to_owned()),
                    class: a.class,
                    name: seen.and_then(|v| v.name.clone()),
                    arg_count: seen.and_then(|v| v.arg_count).unwrap_or(0),
                    keep_running: false,
                    ports: seen.map(|v| v.ports.clone()).unwrap_or_default(),
                });
            self.note_left_running(entry);
        }
    }

    /// Stops one process a deleted session left running, after the user asked for it (the after-delete question
    /// or the list's Stop). Refused unless `target` is exactly an entry of the list; the stop runs on that
    /// session's authority with the recorded identity as a fact (module invariants). The entry leaves the list
    /// when the process is gone afterwards. Blocking (scan, TERM -> wait -> KILL).
    ///
    /// # Errors
    /// [`StopRefusal::NotLeftRunning`] for an identity that is not on the list; any other refusal of
    /// [`stop`] (pid reused, a group shared with another owner); `Scan` / `Signal` on OS failures.
    pub fn stop_left_running(
        &self,
        target: ProcessRef,
        sessions: &[SessionContext],
    ) -> Result<StopOutcome, StopError> {
        let (entry, all) = {
            let left = self.lock_left();
            let entry = left.iter().find(|e| e.process == target).cloned();
            (entry, left.clone())
        };
        let Some(entry) = entry else {
            return Err(StopRefusal::NotLeftRunning { pid: target.pid }.into());
        };
        let session = entry.session_id.clone().unwrap_or_default();
        let mut inputs = self.inputs(sessions);
        // Every listed identity is a recorded fact of its deleted session (their descendants with it), so a
        // group of two of its servers stops together, and anything else in the group refuses the stop.
        inputs.adopted.extend(all.iter().filter_map(|e| {
            Some(Adopted {
                session_id: e.session_id.clone()?,
                process: e.process,
                class: ProcessClass::Agent,
            })
        }));
        let outcome = stop(
            target,
            &Authority::Session(session),
            &inputs,
            &self.inner.config.stop,
        )?;
        if outcome.gone {
            self.lock_left().retain(|e| e.process != target);
        }
        Ok(outcome)
    }
}
