//! Secret-use metadata log and login-fill counters, kept only for this app run.
//! Depends on the Settings views and fill counters; called by the vault broker, fills and Settings.
//! Values are never logged; the newest 200 uses are retained.
use crate::secrets_view::SecretUseView;
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

pub(crate) const MAX_LOG: usize = 200;
#[derive(Default)]
pub(crate) struct SecretUses {
    log: Mutex<VecDeque<SecretUseView>>,
    pub(crate) fill_uses: crate::fill_grants::FillUses,
}
impl SecretUses {
    /// Appends one log entry (newest first; the oldest past [`MAX_LOG`] is dropped).
    pub(crate) fn log(&self, entry: SecretUseView) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.push_front(entry);
        log.truncate(MAX_LOG);
    }

    /// The log of this run, newest first.
    pub(crate) fn log_entries(&self) -> Vec<SecretUseView> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }
}
