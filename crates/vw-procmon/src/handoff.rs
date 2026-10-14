//! A session id hand-off in the monitor: Vibe moved a conversation to a new session id
//! (`/clear`, a rewind into a new session, a compaction with a new id), so everything the monitor keys by session
//! follows it, in memory and on disk.
//!
//! Depends on: `monitor` (the shared state), `registry`, `leases`, `ledger`. Called by: vw-core-api `handoff`.
//! Invariants:
//! - Nothing is stopped or started: the same processes and ports, now owned by the new id, so the agent of the
//!   new session can list, stop and restart what it started before the hand-off, and its `PORT` stays leased.
//! - The ledger on disk is rewritten with the new id, so a crash right after the hand-off re-attaches the
//!   processes to the session the window shows.

use crate::error::LedgerError;
use crate::monitor::ProcMon;

/// What moved in a hand-off (for the log and tests).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HandOffReport {
    /// Registrations (our terminals' shells, `process_start` processes) that moved.
    pub registrations: usize,
    /// Leased ports that moved.
    pub ports: Vec<u16>,
    /// Re-attached processes (from an earlier run) that moved.
    pub adopted: usize,
    /// Ledger entries rewritten.
    pub ledger_entries: usize,
}

impl ProcMon {
    /// Moves session `old`'s registrations, re-attached processes, lease, output notes and ledger entries to
    /// `new`. Blocking (writes the ledger file when one is configured).
    ///
    /// # Errors
    /// [`LedgerError`] when the ledger file cannot be rewritten; the in-memory move has happened by then.
    pub fn session_handed_off(&self, old: &str, new: &str) -> Result<HandOffReport, LedgerError> {
        let mut report = HandOffReport {
            registrations: self.inner.registry.hand_off(old, new),
            ports: self.inner.leases.hand_off(old, new),
            ..HandOffReport::default()
        };
        {
            let mut adopted = self
                .inner
                .adopted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for item in adopted.iter_mut().filter(|a| a.session_id == old) {
                new.clone_into(&mut item.session_id);
                report.adopted += 1;
            }
        }
        {
            let mut outputs = self
                .inner
                .outputs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let moved: Vec<((String, String), String)> = outputs
                .iter()
                .filter(|((session, _), _)| session == old)
                .map(|((_, process), text)| ((new.to_owned(), process.clone()), text.clone()))
                .collect();
            outputs.retain(|(session, _), _| session != old);
            outputs.extend(moved);
        }
        if let Some(ledger) = self.inner.ledger.as_ref() {
            let mut entries = ledger.entries();
            for entry in entries
                .iter_mut()
                .filter(|e| e.session_id.as_deref() == Some(old))
            {
                entry.session_id = Some(new.to_owned());
                report.ledger_entries += 1;
            }
            if report.ledger_entries > 0 {
                ledger.replace(entries)?;
            }
        }
        Ok(report)
    }
}
