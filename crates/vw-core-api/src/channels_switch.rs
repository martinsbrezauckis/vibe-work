//! Which open channels follow the latest Developer permissions switch (`switch_apply.rs`).
//! Depends on parent-private registry state; called by the switch and by the before-work check.
//! A channel remembers the generation read before its open request was sent, so an open that races a switch
//! counts as stale (one extra apply, never a missed one). Closing the channel drops the mark with the slot.
use super::{Channels, locked};
use std::sync::atomic::Ordering;

impl Channels {
    /// The current Developer-switch generation.
    pub(crate) fn switch_generation(&self) -> u64 {
        self.switch_generation.load(Ordering::Acquire)
    }

    /// A switch changed command servers on disk: every channel opened before now is stale. Called under the
    /// Developer-permissions mutation lock, after the config was written.
    pub(crate) fn bump_switch_generation(&self) -> u64 {
        self.switch_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// True when `id` has a live channel whose runtime may still follow an older switch state.
    pub(crate) fn switch_stale(&self, id: &str) -> bool {
        let current = self.switch_generation();
        locked(&self.inner)
            .slots
            .get(id)
            .is_some_and(|slot| slot.alive() && slot.switch_applied < current)
    }

    /// The channel of `id` verifiably follows generation `generation` now.
    pub(crate) fn switch_applied(&self, id: &str, generation: u64) {
        if let Some(slot) = locked(&self.inner).slots.get_mut(id) {
            slot.switch_applied = slot.switch_applied.max(generation);
        }
    }
}
