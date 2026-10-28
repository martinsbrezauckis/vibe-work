//! Deferred channel closure and idle selection for the channel registry.
//! Depends on parent-private registry state; called by recovery and the serialized before-work refresh.
use super::{Channels, locked};
use std::collections::BTreeSet;
use std::time::Duration;

impl Channels {
    /// A requested runtime refresh must finish before the next work, even in a viewed chat.
    pub(crate) fn closing_when_quiet(&self, id: &str) -> bool {
        locked(&self.inner).close_when_quiet.contains(id)
    }

    /// Closes the session's channel as soon as it is quiet (archive or runtime refresh).
    pub(crate) fn close_when_quiet(&self, id: &str) {
        if self.has_channel(id) {
            locked(&self.inner).close_when_quiet.insert(id.to_owned());
        }
    }

    /// Quiet sessions whose refresh or archive is pending, or which are unviewed and unused for `idle`.
    pub(crate) fn idle(&self, idle: Duration, quiet: impl Fn(&str) -> bool) -> Vec<String> {
        let candidates: Vec<String> = {
            let inner = locked(&self.inner);
            let viewed: BTreeSet<&String> = inner.viewed.values().collect();
            inner
                .slots
                .iter()
                .filter(|(id, slot)| {
                    slot.alive()
                        && (inner.close_when_quiet.contains(*id)
                            || (!viewed.contains(id) && slot.last_used.elapsed() >= idle))
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        // `quiet` reads the store and the gate: called without the registry lock held.
        candidates.into_iter().filter(|id| quiet(id)).collect()
    }
}
