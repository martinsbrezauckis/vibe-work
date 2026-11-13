//! The core's side of the Changes tab: one tracker per session (its review
//! baseline, file watcher and turn labels), the baseline taken before a session's work starts, and the task
//! that refreshes on file and effect events and tells subscribers (`changesUpdated`).
//!
//! Depends on: vw-changes (git review), `changes_facts` (session state), `changes_view` (wire types),
//! `core::Shared`, `subscription`, `event`. Called by: `dispatch` (`before_work` hook, the `changes`
//! command through `changes_ops`), `core` (start, drop).
//! Invariants:
//! - Git work runs on `spawn_blocking`, never on the async runtime, holding only that session's tracker.
//! - The baseline exists before Vibe receives the session's first `turn/start`, `session/turn/enqueue` or
//!   `session/shellCommand` (`before_work` runs inside `forward`, before the gate call). A baseline that
//!   takes longer than `CoreConfig::changes_baseline_limit` is discarded (taken late, it could contain the agent's first edits)
//!   and the review is reported unavailable.
//! - A failure here never blocks a turn: it is logged and shown in the tab.

#[path = "changes_refresh.rs"]
mod changes_refresh;
use changes_refresh::run;
pub(crate) use changes_refresh::{build_view, refresh_and_emit};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;
use vw_changes::{Baseline, Debounce, Diff, SessionDir, TurnLabel, Watcher};

use crate::changes_slots::{ChangesTestHooks, Claim, Slots};
use crate::changes_view::{ChangesMode, ChangesView};
use crate::core::Shared;
use crate::event::CoreEvent;
use crate::subscription::CoreSubscription;

/// Default of `CoreConfig::changes_baseline_limit`: longest a baseline may take before the session's first
/// turn is sent without one.
pub(crate) const BASELINE_LIMIT: Duration = Duration::from_secs(15);

/// Methods that start work in a session folder: the baseline must exist before Vibe gets them.
pub(crate) fn starts_work(method: &str) -> bool {
    matches!(
        method,
        "turn/start" | "session/turn/enqueue" | "session/shellCommand"
    )
}

/// The git review of a tracked session.
pub(crate) enum Review {
    /// A baseline (and a watcher when it could start).
    Git {
        baseline: Box<Baseline>,
        watcher: Option<Watcher>,
    },
    /// A git folder whose review cannot run.
    Unavailable(String),
    /// The session's baseline was lost or tampered with: changes are shown against `HEAD` (read only) until
    /// the user starts reviewing again (`restart`). Never replaced silently.
    Lost {
        /// Why, for the user.
        reason: String,
        /// The read-only "against the last commit" view (`None` when even that failed).
        against_head: Option<Box<Baseline>>,
    },
}

/// One session's Changes state.
pub(crate) struct Tracker {
    /// The folder the review belongs to.
    pub(crate) folder: PathBuf,
    /// `None`: not a git folder (effects mode).
    pub(crate) review: Option<Review>,
    /// The last refresh's fingerprint (events are sent only when it changes).
    pub(crate) fingerprint: String,
    /// Files first seen changed while a turn ran: path -> (blob then, label).
    pub(crate) observed: HashMap<String, (Option<String>, TurnLabel)>,
    /// The last diff (folder sessions, events).
    pub(crate) last: Option<Diff>,
    /// Whether `review` was looked up (a stored baseline loaded, git asked): an unopened tracker was just
    /// created and is opened by the first `ensure` that holds its lock.
    pub(crate) opened: bool,
    /// Nearest Git entry at the last discovery, including a parent repository.
    git_entry: Option<PathBuf>,
}

// This is a cheap rediscovery hint; Git itself still decides the repository and scope.
fn nearest_git_entry(folder: &Path) -> Option<PathBuf> {
    folder.ancestors().find_map(|parent| {
        let entry = parent.join(".git");
        entry.symlink_metadata().is_ok().then_some(entry)
    })
}

impl Tracker {
    /// A tracker for `folder` that has not looked for its review yet.
    pub(crate) fn unopened(folder: &Path) -> Self {
        Self {
            folder: folder.to_path_buf(),
            review: None,
            fingerprint: String::new(),
            observed: HashMap::new(),
            last: None,
            opened: false,
            git_entry: nearest_git_entry(folder),
        }
    }
}

/// All trackers plus the refresh task.
pub(crate) struct Hub {
    pub(crate) dir: Option<PathBuf>,
    /// The diff line comments, in `<dir>/line-comments`; `None` with `dir`.
    pub(crate) comments: Option<Arc<vw_changes::LineCommentStore>>,
    /// Trackers, generations and claims (`changes_slots`).
    pub(crate) slots: Mutex<Slots>,
    /// `CoreConfig::changes_test_hooks` (tests only).
    hooks: Option<Arc<dyn ChangesTestHooks>>,
    signals: mpsc::UnboundedSender<String>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    stop: Arc<tokio::sync::Notify>,
}

impl Drop for Hub {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl Hub {
    /// The hub `config` asks for (`changes_dir`, `changes_test_hooks`).
    pub(crate) fn from_config(config: &crate::core::CoreConfig) -> Self {
        Self::new(
            config.changes_dir.clone(),
            config.changes_test_hooks.clone(),
        )
    }

    /// A hub keeping its data in `dir` (`None`: the Changes tab is off).
    pub(crate) fn new(dir: Option<PathBuf>, hooks: Option<Arc<dyn ChangesTestHooks>>) -> Self {
        if let Some(dir) = &dir
            && let Err(error) = vw_changes::prepare_root(dir)
        {
            warn!(%error, "changes: data folder not usable; the Changes tab is off");
            return Self::new(None, hooks);
        }
        let (signals, receiver) = mpsc::unbounded_channel();
        let comments = dir
            .as_ref()
            .map(|d| Arc::new(vw_changes::LineCommentStore::new(d.join("line-comments"))));
        Self {
            dir,
            comments,
            slots: Mutex::new(Slots::default()),
            hooks,
            signals,
            receiver: Mutex::new(Some(receiver)),
            task: Mutex::new(None),
            stop: Arc::default(),
        }
    }

    /// Stops scheduling refreshes and joins the current refresh, including its blocking git work.
    pub(crate) async fn shutdown(&self) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.stop.notify_one();
        if let Some(task) = task {
            let _ = task.await;
        }
        // Watchers can be reading git's ignored list on their own threads. Drop and join them off the runtime.
        let trackers = self.watch_trackers();
        let _ = tokio::task::spawn_blocking(move || {
            for tracker in trackers {
                let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(Review::Git { watcher, .. }) = &mut guard.review {
                    drop(watcher.take());
                }
            }
        })
        .await;
    }

    /// Blocking: the tracker of `session_id` for `folder`, created when missing and opened once (a stored
    /// baseline is loaded). With `claim` (the caller may write the session's baseline folder) a missing
    /// baseline is taken. A moved folder (a Vibe worktree) gets a baseline of its `HEAD` when the folder can
    /// be claimed. A result whose generation ended meanwhile is dropped; a dropped baseline the caller wrote
    /// takes the session's baseline folder with it (`changes_slots`).
    pub(crate) fn ensure(
        &self,
        session_id: &str,
        folder: &Path,
        claim: Option<&Claim>,
    ) -> Arc<Mutex<Tracker>> {
        // A reader that lost a race with a newer generation looks again (bounded: generations change only on
        // expiry, deletion and restart).
        for _ in 0..3 {
            let (tracker, generation) = self.get_or_insert(session_id, folder);
            if claim.is_some_and(|c| c.generation != generation) {
                // Claimed in a generation that has ended (the deadline passed before this ran): its work
                // would be late, so it does nothing.
                return tracker;
            }
            let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
            let moved = guard.opened && guard.folder != folder;
            let git_entry = nearest_git_entry(folder);
            let initialized = guard.opened
                && git_entry.is_some()
                && guard.git_entry != git_entry
                && !matches!(guard.review, Some(Review::Lost { .. }));
            let rediscover = moved || initialized;
            let take = claim.is_some()
                && !matches!(guard.review, Some(Review::Git { .. } | Review::Lost { .. }));
            if guard.opened && !rediscover && !take {
                drop(guard);
                return tracker;
            }
            // Only a claim holder writes the baseline folder; a reader that sees a move claims it itself, and
            // never loads a stored baseline while someone else is writing it.
            let own_claim = if claim.is_none() && rediscover {
                self.claim(session_id)
            } else {
                None
            };
            if claim.is_none() && own_claim.is_none() && (rediscover || self.claimed(session_id)) {
                drop(guard);
                return tracker;
            }
            let writer = claim.is_some() || own_claim.is_some();
            let review = if rediscover {
                // The session moved on its first turn in a Vibe worktree: review the new folder from
                // its HEAD (a fresh checkout), never against the old folder's baseline.
                // A newly initialized repository also starts at HEAD so existing edits remain visible.
                self.open_review(session_id, folder, true, true)
            } else {
                self.open_review(session_id, folder, claim.is_some(), false)
            };
            if writer && let Some(hooks) = &self.hooks {
                hooks.before_install(session_id);
            }
            let installed = self.install(session_id, generation, &tracker, writer, || {
                if rediscover {
                    guard.observed.clear();
                    guard.last = None;
                }
                guard.folder = folder.to_path_buf();
                guard.review = review;
                guard.opened = true;
                guard.git_entry = git_entry;
            });
            drop(guard);
            if installed {
                return tracker;
            }
            if writer {
                // Taken for a generation that ended (the deadline passed, the session was deleted): it may
                // hold the agent's first edits, so neither it nor its folder survives.
                warn!(
                    session_id,
                    "changes: a baseline that ended too late was discarded"
                );
                self.remove_data(session_id);
                return self.get_or_insert(session_id, folder).0;
            }
        }
        self.get_or_insert(session_id, folder).0
    }

    /// Blocking: the review for `folder`: `None` without git; a loaded or new baseline; `Unavailable` with
    /// the reason when git refuses.
    fn open_review(
        &self,
        session_id: &str,
        folder: &Path,
        take: bool,
        from_head: bool,
    ) -> Option<Review> {
        let Some(dir) = &self.dir else {
            return Some(Review::Unavailable(
                "the app has no data folder for reviews".into(),
            ));
        };
        let repo = match vw_changes::discover(folder) {
            Ok(Some(repo)) => repo,
            Ok(None) => return None,
            Err(error) => return Some(Review::Unavailable(error.to_string())),
        };
        let session_dir = match SessionDir::open(dir, session_id) {
            Ok(d) => d,
            Err(error) => return Some(Review::Unavailable(error.to_string())),
        };
        let loaded = if from_head {
            vw_changes::Loaded::None
        } else {
            Baseline::open(repo.clone(), session_dir.clone())
        };
        let baseline = match loaded {
            vw_changes::Loaded::Ready(b) => *b,
            vw_changes::Loaded::Lost(reason) => {
                warn!(session_id, %reason, "changes: the baseline is lost; showing changes against HEAD");
                return Some(lost_review(repo, &session_dir, reason));
            }
            vw_changes::Loaded::None if !take && !from_head => return None,
            vw_changes::Loaded::None => {
                let made = if from_head {
                    Baseline::from_head(repo, session_dir)
                } else {
                    Baseline::take(repo, session_dir)
                };
                match made {
                    Ok(b) => b,
                    Err(error) => {
                        warn!(session_id, %error, "changes: baseline not taken");
                        return Some(Review::Unavailable(error.to_string()));
                    }
                }
            }
        };
        let watcher = self.start_watcher(session_id, &baseline);
        Some(Review::Git {
            baseline: Box::new(baseline),
            watcher,
        })
    }

    /// Blocking: takes a new baseline for `session_id` now (the user's explicit "start reviewing from now"),
    /// in a new generation: anything still in flight for the old one is dropped when it ends. `_claim`: the
    /// caller holds the session's claim (it waited for any baseline still being taken).
    pub(crate) fn restart(
        &self,
        session_id: &str,
        folder: &Path,
        _claim: &Claim,
    ) -> Arc<Mutex<Tracker>> {
        self.renew(session_id);
        let (tracker, generation) = self.get_or_insert(session_id, folder);
        let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = self.dir.as_ref().and_then(|dir| {
            let repo = vw_changes::discover(folder).ok().flatten()?;
            let session_dir = SessionDir::open(dir, session_id).ok()?;
            Some(match Baseline::take(repo, session_dir) {
                Ok(baseline) => {
                    let watcher = self.start_watcher(session_id, &baseline);
                    Review::Git {
                        baseline: Box::new(baseline),
                        watcher,
                    }
                }
                Err(error) => Review::Unavailable(error.to_string()),
            })
        });
        let Some(review) = fresh else {
            // Not a git folder (or no data folder): nothing to restart; the normal lookup.
            drop(guard);
            return self.ensure(session_id, folder, None);
        };
        let installed = self.install(session_id, generation, &tracker, true, || {
            guard.review = Some(review);
            guard.folder = folder.to_path_buf();
            guard.opened = true;
            guard.git_entry = nearest_git_entry(folder);
            guard.observed.clear();
            guard.last = None;
        });
        drop(guard);
        if !installed {
            // The session was deleted while the baseline was taken.
            self.remove_data(session_id);
        }
        tracker
    }

    /// Asks the refresh loop to read `session_id` again now (after a hand-off moved a baseline to it).
    pub(crate) fn request_refresh(&self, session_id: &str) {
        let _ = self.signals.send(session_id.to_owned());
    }

    fn start_watcher(&self, session_id: &str, baseline: &Baseline) -> Option<Watcher> {
        let repo = baseline.repo().clone();
        let session_dir = baseline.dir().clone();
        let signals = self.signals.clone();
        let id = session_id.to_owned();
        let for_ignored = repo.clone();
        // Paths the watcher may drop must be ignored by the baseline's rules too: an ignore rule the agent
        // adds never silences the live refresh (review integrity).
        let watcher = vw_changes::watch(
            repo.root.clone(),
            repo.git_dir.clone(),
            Debounce::default(),
            move || vw_changes::watch_ignored(&for_ignored, &session_dir).unwrap_or_default(),
            move || {
                let _ = signals.send(id.clone());
            },
        );
        match watcher {
            Ok(w) => Some(w),
            Err(error) => {
                warn!(session_id, %error, "changes: no file watcher; refreshes come from Vibe's events");
                None
            }
        }
    }

    /// Starts the refresh task (once).
    pub(crate) fn spawn(&self, shared: &Arc<Shared>) {
        let Some(receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        if self.dir.is_none() {
            return;
        }
        let subscription = CoreSubscription::new(shared, None);
        let task = tokio::spawn(run(
            Arc::downgrade(shared),
            subscription,
            receiver,
            self.stop.clone(),
        ));
        *self.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
    }
}

/// The read-only "against the last commit" review of a session whose baseline was lost.
fn lost_review(repo: vw_changes::Repo, session_dir: &SessionDir, reason: String) -> Review {
    let against_head = match vw_changes::against_head(repo, session_dir) {
        Ok(view) => Some(Box::new(view)),
        Err(error) => {
            warn!(%error, "changes: no view against HEAD either");
            None
        }
    };
    Review::Lost {
        reason,
        against_head,
    }
}

#[cfg(test)]
#[path = "changes_shutdown_tests.rs"]
mod shutdown_tests;

#[cfg(test)]
#[path = "changes_deadline_tests.rs"]
mod deadline_tests;

/// Sends `changesUpdated` for `view` when `changed`. Every refresh reports its own change, whoever asked
/// for it (a client's read must not swallow the event other clients wait for).
pub(crate) fn emit_if_changed(shared: &Shared, changed: bool, view: &ChangesView) {
    if !changed {
        return;
    }
    shared.emit(CoreEvent::ChangesUpdated {
        session_id: view.session_id.clone(),
        mode: view.mode,
        files: u32::try_from(if view.mode == ChangesMode::Effects {
            view.turns.iter().map(|t| t.files.len()).sum::<usize>()
        } else {
            view.files.len()
        })
        .unwrap_or(u32::MAX),
        fingerprint: view.fingerprint.clone(),
    });
}

#[cfg(test)]
#[path = "changes_attribution_tests.rs"]
mod attribution_tests;
