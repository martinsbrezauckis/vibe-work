//! The documents each mount's frames were allowed to load, in order, as the engine asked the navigation hooks about
//! them (`allow_navigation`, `allow_frame_navigation`, and a document `request_check`), and the new windows its page
//! asked for (`new_window_requested`), whatever became of them. A browser tool that runs page
//! code marks the journal before and reads it after, so a frame document that came and went while the code ran is
//! still known (a page script can load a document into a frame, read it and remove the frame before it returns).
//! Bounded: per mount the newest rows, and a fixed number of mounts; a reader whose rows were dropped gets `None` and
//! must treat the run as unproven.
use std::collections::{BTreeMap, VecDeque};

use url::Url;

use crate::{MountId, PreviewService, hooks::Hooks};

const ROWS: usize = 64;
const MOUNTS: usize = 256;

#[derive(Default)]
pub(crate) struct Journal {
    /// The sequence number of the newest row of any mount.
    seq: u64,
    /// Rows at or below this number may be missing from a mount that is no longer listed.
    forgotten_through: u64,
    mounts: BTreeMap<String, Entry>,
}

/// A document a mount's frame was allowed to load, or a new window its page asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedDocument {
    /// The address asked for.
    pub url: Url,
    /// A new window (always denied natively; the host may open the address as a tab of its own).
    pub window: bool,
}

#[derive(Default)]
struct Entry {
    rows: VecDeque<(u64, LoadedDocument)>,
    /// The newest row dropped from this mount for the bound.
    dropped_through: u64,
}

impl Journal {
    fn record(&mut self, label: &str, url: &Url, window: bool) {
        self.seq += 1;
        if !self.mounts.contains_key(label) && self.mounts.len() >= MOUNTS {
            let oldest = self
                .mounts
                .iter()
                .min_by_key(|(_, entry)| entry.rows.back().map_or(0, |row| row.0))
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.mounts.remove(&oldest);
                self.forgotten_through = self.seq;
            }
        }
        let entry = self.mounts.entry(label.to_owned()).or_default();
        entry.rows.push_back((
            self.seq,
            LoadedDocument {
                url: url.clone(),
                window,
            },
        ));
        if entry.rows.len() > ROWS
            && let Some((seq, _)) = entry.rows.pop_front()
        {
            entry.dropped_through = seq;
        }
    }

    fn since(&self, label: &str, mark: u64) -> Option<Vec<LoadedDocument>> {
        let Some(entry) = self.mounts.get(label) else {
            return (self.forgotten_through <= mark).then(Vec::new);
        };
        if entry.dropped_through > mark {
            return None;
        }
        Some(
            entry
                .rows
                .iter()
                .filter(|(seq, _)| *seq > mark)
                .map(|(_, row)| row.clone())
                .collect(),
        )
    }
}

impl Hooks {
    /// Notes a document the engine was allowed to load in one of `label`'s frames.
    pub(crate) fn document_allowed(&self, label: &str, url: &Url) {
        self.journal(label, url, false);
    }

    /// Notes a new window `label`'s page asked for.
    pub(crate) fn window_requested(&self, label: &str, url: &Url) {
        self.journal(label, url, true);
    }

    fn journal(&self, label: &str, url: &Url, window: bool) {
        self.0
            .documents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(label, url, window);
    }
}

impl PreviewService {
    /// A position in the document journal: [`Self::documents_since`] lists what loads after it.
    #[must_use]
    pub fn document_mark(&self) -> u64 {
        self.inner
            .documents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seq
    }

    /// The documents `id`'s frames (its page included) were allowed to load, and the windows its page asked for,
    /// after `mark`, oldest first. `None`
    /// when some of them are no longer known (bounded journal): the caller cannot prove what loaded.
    #[must_use]
    pub fn documents_since(&self, id: &MountId, mark: u64) -> Option<Vec<LoadedDocument>> {
        self.inner
            .documents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .since(&id.label, mark)
    }
}

#[cfg(test)]
mod tests {
    use super::Journal;

    #[test]
    fn rows_after_a_mark_and_dropped_rows_are_unproven() {
        let url = |path: &str| url::Url::parse(&format!("https://site.test/{path}")).unwrap();
        let mut journal = Journal::default();
        journal.record("a", &url("before"), false);
        let mark = journal.seq;
        assert_eq!(journal.since("a", mark), Some(vec![]));
        journal.record("b", &url("other"), false);
        journal.record("a", &url("frame"), false);
        journal.record("a", &url("window"), true);
        let rows = journal.since("a", mark).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].url.clone(), rows[0].window), (url("frame"), false));
        assert_eq!((rows[1].url.clone(), rows[1].window), (url("window"), true));
        assert_eq!(journal.since("missing", mark), Some(vec![]));
        for n in 0..super::ROWS {
            journal.record("a", &url(&n.to_string()), false);
        }
        assert_eq!(journal.since("a", mark), None);
        assert!(journal.since("a", journal.seq).is_some());
    }

    #[test]
    fn a_forgotten_mount_is_unproven() {
        let mut journal = Journal::default();
        let url = url::Url::parse("https://site.test/").unwrap();
        journal.record("first", &url, false);
        let mark = journal.seq;
        journal.record("first", &url, false);
        for n in 0..super::MOUNTS {
            journal.record(&format!("m{n}"), &url, false);
        }
        assert_eq!(journal.since("first", mark), None);
    }
}
