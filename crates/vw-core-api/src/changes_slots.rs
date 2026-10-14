//! Per-session bookkeeping of the Changes hub: which tracker is current, the
//! session's generation, and who may write the session's baseline folder.
//!
//! Depends on: `changes` (`Hub`, `Tracker`, `Review`), tokio `watch`, vw-changes `SessionDir`. Called by:
//! `changes` (`ensure`, `restart`, `forget`), `changes_ops` (the baseline before work, restart,
//! enableHistory), `handoff` (a session id hand-off moves the baseline and the comments).
//! Invariants:
//! - A tracker is created atomically (get-or-insert under the slots lock) and opened once under its own lock,
//!   so concurrent callers share one tracker; no caller's tracker silently replaces another's. Before, two
//!   callers could each build a tracker and the last insert won: a read overwrote "unavailable" with "not
//!   git", and a baseline that ended after its deadline overwrote "unavailable" with itself.
//! - Lock order: a tracker's lock, then the slots lock; never the other way round.
//! - Generation: bumped when a session's review is invalidated (its baseline missed the deadline, the
//!   session was deleted, the user starts reviewing again). A result is installed only under the slots lock
//!   and only if the generation it started in is still current; otherwise it is dropped (its watcher stops)
//!   and, when its caller was writing the baseline folder, that folder is removed.
//! - At most one writer of a session's baseline folder at a time: the holder of the session's `Claim`.
//!   Taking, moving and restarting a baseline need it; reads never write the folder and never load it while
//!   someone else holds the claim. Dropping the claim releases it (also on a panic).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;
use tokio::time::Instant;
use tracing::warn;
use vw_changes::SessionDir;

use crate::changes::{Hub, Review, Tracker};

/// The hub's per-session maps, behind one lock.
#[derive(Default)]
pub(crate) struct Slots {
    /// The current tracker of each session.
    trackers: HashMap<String, Arc<Mutex<Tracker>>>,
    /// Each session's generation (absent: 0).
    generations: HashMap<String, u64>,
    /// The last generation handed out (shared by all sessions, so a number is never reused).
    next: u64,
    /// The receiver of each session's claim; the claim is held while its sender lives.
    claims: HashMap<String, watch::Receiver<()>>,
}

impl Slots {
    fn generation(&self, session_id: &str) -> u64 {
        self.generations.get(session_id).copied().unwrap_or(0)
    }

    fn bump(&mut self, session_id: &str) -> u64 {
        self.next += 1;
        self.generations.insert(session_id.to_owned(), self.next);
        self.next
    }

    /// The receiver of a claim that is still held.
    fn held(&self, session_id: &str) -> Option<watch::Receiver<()>> {
        self.claims
            .get(session_id)
            .filter(|rx| rx.has_changed().is_ok())
            .cloned()
    }
}

/// Test hooks around the baseline taken before a session's first work (`CoreConfig::changes_test_hooks`).
/// Both run on that baseline's blocking thread; production cores have none.
pub trait ChangesTestHooks: Send + Sync {
    /// Before the baseline is looked up or taken (blocking here makes it end past the deadline).
    fn before_take(&self, _session_id: &str) {}
    /// Inside the take, after the baseline was written and before it is installed (blocking here makes a
    /// baseline that was taken in time end past the deadline). Called for every baseline writer.
    fn before_install(&self, _session_id: &str) {}
    /// After its result was installed or dropped (a dropped baseline's folder is already removed).
    fn after_take(&self, _session_id: &str) {}
}

/// The right to write one session's baseline folder; released when dropped.
#[must_use = "dropping a claim releases it"]
pub(crate) struct Claim {
    _held: watch::Sender<()>,
    /// The session's generation when the claim was taken: a baseline taken under this claim belongs to it,
    /// so work that starts after the generation ended (the deadline passed before its thread ran) does
    /// nothing on Windows.
    pub(crate) generation: u64,
}

impl Hub {
    pub(crate) fn slots(&self) -> MutexGuard<'_, Slots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The current tracker of `session_id`, if one exists.
    pub(crate) fn tracker(&self, session_id: &str) -> Option<Arc<Mutex<Tracker>>> {
        self.slots().trackers.get(session_id).cloned()
    }

    /// The current tracker of `session_id` (an unopened one for `folder` when missing) and the session's
    /// generation, read together.
    pub(crate) fn get_or_insert(
        &self,
        session_id: &str,
        folder: &Path,
    ) -> (Arc<Mutex<Tracker>>, u64) {
        let mut slots = self.slots();
        let generation = slots.generation(session_id);
        let tracker = slots
            .trackers
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(Tracker::unopened(folder))))
            .clone();
        (tracker, generation)
    }

    /// Runs `apply` (the caller holds `tracker`'s lock) under the slots lock if `generation` is still the
    /// session's, and makes `tracker` the current one. A reader (`writer: false`) never replaces a different
    /// current tracker. Returns whether `apply` ran; when it did not, everything it captured is dropped.
    pub(crate) fn install(
        &self,
        session_id: &str,
        generation: u64,
        tracker: &Arc<Mutex<Tracker>>,
        writer: bool,
        apply: impl FnOnce(),
    ) -> bool {
        let mut slots = self.slots();
        if slots.generation(session_id) != generation {
            return false;
        }
        match slots.trackers.get(session_id) {
            Some(current) if Arc::ptr_eq(current, tracker) => {}
            Some(_) if !writer => return false,
            _ => {
                slots
                    .trackers
                    .insert(session_id.to_owned(), tracker.clone());
            }
        }
        apply();
        true
    }

    /// The session's claim if nobody holds it.
    pub(crate) fn claim(&self, session_id: &str) -> Option<Claim> {
        self.claim_or_held(session_id).ok()
    }

    fn claim_or_held(&self, session_id: &str) -> Result<Claim, watch::Receiver<()>> {
        let mut slots = self.slots();
        if let Some(held) = slots.held(session_id) {
            return Err(held);
        }
        let (sender, receiver) = watch::channel(());
        slots.claims.insert(session_id.to_owned(), receiver);
        Ok(Claim {
            _held: sender,
            generation: slots.generation(session_id),
        })
    }

    /// Whether someone holds the session's claim.
    pub(crate) fn claimed(&self, session_id: &str) -> bool {
        self.slots().held(session_id).is_some()
    }

    /// The session's claim, waiting for its holder to release it; `None` when `until` passes first.
    pub(crate) async fn claim_wait(
        &self,
        session_id: &str,
        until: Option<Instant>,
    ) -> Option<Claim> {
        loop {
            let mut held = match self.claim_or_held(session_id) {
                Ok(claim) => return Some(claim),
                Err(held) => held,
            };
            // Nothing is ever sent: `changed` ends when the holder drops the claim.
            let released = held.changed();
            match until {
                Some(until) => {
                    if tokio::time::timeout_at(until, released).await.is_err() {
                        return None;
                    }
                }
                None => {
                    let _ = released.await;
                }
            }
        }
    }

    /// The baseline of `session_id` missed its deadline: a new generation (so the late result is dropped
    /// when it ends) whose tracker says "unavailable" with `reason`. Returns that generation.
    pub(crate) fn expire(&self, session_id: &str, folder: &Path, reason: &str) -> u64 {
        let mut tracker = Tracker::unopened(folder);
        tracker.review = Some(Review::Unavailable(reason.to_owned()));
        tracker.opened = true;
        let mut slots = self.slots();
        let generation = slots.bump(session_id);
        slots
            .trackers
            .insert(session_id.to_owned(), Arc::new(Mutex::new(tracker)));
        generation
    }

    /// A new generation for `session_id` (the user starts reviewing again). Returns it.
    pub(crate) fn renew(&self, session_id: &str) -> u64 {
        self.slots().bump(session_id)
    }

    /// Forgets a session (its watcher stops). With `delete_data` (the session was deleted) its generation
    /// ends too and its baseline folder is removed.
    pub(crate) fn forget(&self, session_id: &str, delete_data: bool) {
        {
            let mut slots = self.slots();
            slots.trackers.remove(session_id);
            if delete_data {
                slots.bump(session_id);
            }
        }
        if delete_data {
            self.remove_data(session_id);
            // Its diff comments go with it.
            if let Some(comments) = &self.comments {
                comments.remove_session(session_id);
            }
        }
    }

    /// Session `old`'s conversation continues as `new` (a hand-off): its tracker stops (the next read of
    /// `new` opens the moved baseline), its generation ends, and its baseline folder and diff comments move to
    /// `new` (blocking). A baseline still being written for `old` (its claim is held) is left where it is: its
    /// writer installs it under `old`'s generation, which this ends, so it is dropped; `new` then has no review
    /// until the user starts one, as after any lost baseline.
    pub(crate) fn hand_off(&self, old: &str, new: &str) {
        let busy = {
            let mut slots = self.slots();
            slots.trackers.remove(old);
            slots.bump(old);
            slots.held(old).is_some()
        };
        if let Some(dir) = &self.dir
            && !busy
            && let Err(error) = SessionDir::rename_existing(dir, old, new)
        {
            warn!(%error, "changes: baseline folder did not follow the hand-off");
        }
        if let Some(comments) = &self.comments
            && let Err(error) = comments.move_session(old, new)
        {
            warn!(%error, "changes: diff comments did not follow the hand-off");
        }
    }

    /// Removes the session's baseline folder (blocking).
    pub(crate) fn remove_data(&self, session_id: &str) {
        if let Some(dir) = &self.dir
            && let Err(error) = SessionDir::remove_existing(dir, session_id)
        {
            warn!(session_id, %error, "changes: baseline folder not removed");
        }
    }

    /// Removes the session's baseline folder if `generation` is still the session's (blocking). Used after
    /// a late baseline ended: whatever it wrote goes, unless the session was reviewed again meanwhile.
    pub(crate) fn remove_data_if(&self, session_id: &str, generation: u64) {
        let current = self.slots().generation(session_id);
        if current == generation {
            self.remove_data(session_id);
        }
    }
}
