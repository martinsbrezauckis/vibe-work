//! Bounded ownership of live Windows toasts, independent of `WinRT` for portable lifetime tests.
//! Called by the Windows notifier; every known event, including expiry, transfers ownership out.

use std::collections::VecDeque;

pub(super) const MAX_TOASTS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ToastEvent {
    Activated,
    UserCanceled,
    Removed,
    TimedOut,
}

pub(super) struct ToastStore<T> {
    entries: VecDeque<(u64, T)>,
}

impl<T> ToastStore<T> {
    pub(super) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    // Return evictions/removals so WinRT unsubscription and user callbacks run without the lock.
    pub(super) fn insert(&mut self, id: u64, toast: T) -> Option<T> {
        self.entries.push_back((id, toast));
        if self.entries.len() > MAX_TOASTS {
            self.entries.pop_front().map(|(_, toast)| toast)
        } else {
            None
        }
    }

    pub(super) fn event(&mut self, id: u64, _event: ToastEvent) -> Option<T> {
        let index = self.entries.iter().position(|(key, _)| *key == id)?;
        self.entries.remove(index).map(|(_, toast)| toast)
    }

    pub(super) fn remove_where(&mut self, matches: impl Fn(&T) -> bool) -> Vec<T> {
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            if matches(&self.entries[index].1) {
                if let Some((_, toast)) = self.entries.remove(index) {
                    removed.push(toast);
                }
            } else {
                index += 1;
            }
        }
        removed
    }
}
