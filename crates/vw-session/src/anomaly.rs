//! Per-session anomaly accounting: things the server did that a perfect stream would not.
//!
//! Responsibility: count and remember (bounded) protocol irregularities so the Debug API and tests
//! can read them later. Anomalies never abort processing; each one has a defined recovery in the
//! reducer. Depends on: nothing. Called by: `reducer`, `store`, read by the Debug API.

use std::collections::VecDeque;

/// How many recent anomaly descriptions are kept per session.
const RECENT_CAP: usize = 64;

/// The kinds of irregularity the store distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnomalyKind {
    /// An event id at or below the last applied one arrived again (ignored). Benign after a
    /// reload, so it is counted but never needs action.
    DuplicateEvent,
    /// An event id skipped ahead; the event is held back and a resync is requested.
    EventGap,
    /// `history/entryAdded` announced an id the session already holds; the
    /// entry is upserted. Vibe's own reducer raises here.
    ReannouncedEntry,
    /// `history/entryUpdated` touched an entry already `completed` (frozen in Vibe's reducer); the
    /// patch is still applied.
    CompletedEntryUpdated,
    /// A patch could not be applied; state left unchanged and a resync requested.
    PatchFailed,
    /// A notification or state payload was missing a field the reducer needs.
    MalformedPayload,
    /// A snapshot or handoff disagreed with its own envelope (session id or watermark).
    InconsistentEnvelope,
    /// A turn running before a resume was absent from the resumed state and was marked
    /// interrupted locally.
    InterruptedTurn,
}

/// Monotonic counters for one session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnomalyCounts {
    /// See [`AnomalyKind::DuplicateEvent`].
    pub duplicate_events: u64,
    /// See [`AnomalyKind::EventGap`].
    pub event_gaps: u64,
    /// See [`AnomalyKind::ReannouncedEntry`].
    pub reannounced_entries: u64,
    /// See [`AnomalyKind::CompletedEntryUpdated`].
    pub completed_entry_updates: u64,
    /// See [`AnomalyKind::PatchFailed`].
    pub patch_failures: u64,
    /// See [`AnomalyKind::MalformedPayload`].
    pub malformed_payloads: u64,
    /// See [`AnomalyKind::InconsistentEnvelope`].
    pub inconsistent_envelopes: u64,
    /// See [`AnomalyKind::InterruptedTurn`].
    pub interrupted_turns: u64,
}

impl AnomalyCounts {
    /// Sum of every counter except benign duplicates.
    #[must_use]
    pub fn significant(&self) -> u64 {
        self.event_gaps
            + self.reannounced_entries
            + self.completed_entry_updates
            + self.patch_failures
            + self.malformed_payloads
            + self.inconsistent_envelopes
            + self.interrupted_turns
    }
}

/// One remembered anomaly with a human-readable description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnomalyRecord {
    /// What kind it was.
    pub kind: AnomalyKind,
    /// Event id involved, when there is one.
    pub event_id: Option<u64>,
    /// Description for logs and the Debug API.
    pub detail: String,
}

/// Counters plus a bounded log of the most recent records.
#[derive(Debug, Clone, Default)]
pub struct Anomalies {
    counts: AnomalyCounts,
    recent: VecDeque<AnomalyRecord>,
}

impl Anomalies {
    /// Records one anomaly.
    pub fn record(&mut self, kind: AnomalyKind, event_id: Option<u64>, detail: impl Into<String>) {
        let c = &mut self.counts;
        let counter = match kind {
            AnomalyKind::DuplicateEvent => &mut c.duplicate_events,
            AnomalyKind::EventGap => &mut c.event_gaps,
            AnomalyKind::ReannouncedEntry => &mut c.reannounced_entries,
            AnomalyKind::CompletedEntryUpdated => &mut c.completed_entry_updates,
            AnomalyKind::PatchFailed => &mut c.patch_failures,
            AnomalyKind::MalformedPayload => &mut c.malformed_payloads,
            AnomalyKind::InconsistentEnvelope => &mut c.inconsistent_envelopes,
            AnomalyKind::InterruptedTurn => &mut c.interrupted_turns,
        };
        *counter += 1;
        // Duplicates are expected after every reload; keep them out of the descriptive log so
        // they cannot push real problems out of the bounded window.
        if kind != AnomalyKind::DuplicateEvent {
            if self.recent.len() == RECENT_CAP {
                self.recent.pop_front();
            }
            self.recent.push_back(AnomalyRecord {
                kind,
                event_id,
                detail: detail.into(),
            });
        }
    }

    /// The counters.
    #[must_use]
    pub fn counts(&self) -> AnomalyCounts {
        self.counts
    }

    /// The most recent non-duplicate records, oldest first.
    #[must_use]
    pub fn recent(&self) -> Vec<AnomalyRecord> {
        self.recent.iter().cloned().collect()
    }
}
