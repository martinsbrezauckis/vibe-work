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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vw_changes::{Baseline, Debounce, Diff, SessionDir, TurnLabel, TurnSource, Watcher};

use crate::changes_facts::{Facts, facts, last_turn_by_path, path_key, shown};
use crate::changes_slots::{ChangesTestHooks, Claim, Slots};
use crate::changes_view::{ChangesMode, ChangesView, TurnChanges, TurnFile};
use crate::core::Shared;
use crate::event::{CoreEvent, SessionChange};
use crate::subscription::CoreSubscription;

/// Default of `CoreConfig::changes_baseline_limit`: longest a baseline may take before the session's first
/// turn is sent without one.
pub(crate) const BASELINE_LIMIT: Duration = Duration::from_secs(15);
/// Delay that merges a burst of effect events into one refresh.
const EFFECT_SETTLE: Duration = Duration::from_millis(150);

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
        }
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
            let take = claim.is_some()
                && !matches!(guard.review, Some(Review::Git { .. } | Review::Lost { .. }));
            if guard.opened && !moved && !take {
                drop(guard);
                return tracker;
            }
            // Only a claim holder writes the baseline folder; a reader that sees a move claims it itself, and
            // never loads a stored baseline while someone else is writing it.
            let own_claim = if claim.is_none() && moved {
                self.claim(session_id)
            } else {
                None
            };
            if claim.is_none() && own_claim.is_none() && (moved || self.claimed(session_id)) {
                drop(guard);
                return tracker;
            }
            let writer = claim.is_some() || own_claim.is_some();
            let review = if moved {
                // The session moved on its first turn in a Vibe worktree: review the new folder from
                // its HEAD (a fresh checkout), never against the old folder's baseline.
                self.open_review(session_id, folder, true, true)
            } else {
                self.open_review(session_id, folder, claim.is_some(), false)
            };
            if writer && let Some(hooks) = &self.hooks {
                hooks.before_install(session_id);
            }
            let installed = self.install(session_id, generation, &tracker, writer, || {
                if moved {
                    guard.observed.clear();
                    guard.last = None;
                }
                guard.folder = folder.to_path_buf();
                guard.review = review;
                guard.opened = true;
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
        let task = tokio::spawn(run(Arc::downgrade(shared), subscription, receiver));
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

/// The refresh loop: effect and folder events (settled) and watcher signals -> refresh -> event.
async fn run(
    weak: Weak<Shared>,
    mut subscription: CoreSubscription,
    mut signals: mpsc::UnboundedReceiver<String>,
) {
    let mut due: HashMap<String, Instant> = HashMap::new();
    loop {
        let next = due.values().min().copied();
        tokio::select! {
            event = subscription.next() => {
                let Some(event) = event else { break };
                let Some(shared) = weak.upgrade() else { break };
                match interesting(&event) {
                    Interest::Refresh(id) => {
                        due.entry(id).or_insert_with(|| Instant::now() + EFFECT_SETTLE);
                    }
                    Interest::Removed(id) => shared.changes.forget(&id, false),
                    Interest::None => {}
                }
            }
            signal = signals.recv() => {
                let Some(id) = signal else { break };
                due.insert(id, Instant::now());
            }
            () = sleep_until(next) => {
                let now = Instant::now();
                let ready: Vec<String> = due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| id.clone()).collect();
                for id in ready {
                    due.remove(&id);
                    let Some(shared) = weak.upgrade() else { return };
                    refresh_and_emit(&shared, &id).await;
                }
            }
        }
    }
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}

enum Interest {
    Refresh(String),
    Removed(String),
    None,
}

/// Which session an event should refresh: completed effects (a write, an edit, a command) and folder moves.
fn interesting(event: &CoreEvent) -> Interest {
    let CoreEvent::Change { change } = event else {
        return Interest::None;
    };
    match change {
        SessionChange::Removed { session_id } => Interest::Removed(session_id.clone()),
        SessionChange::EntryUpserted {
            session_id, entry, ..
        } => {
            let effect = entry.get("type").and_then(Value::as_str) == Some("effect");
            if effect {
                Interest::Refresh(session_id.clone())
            } else {
                Interest::None
            }
        }
        SessionChange::EntryPatched {
            session_id, patch, ..
        } => {
            let touches_state = patch.iter().any(|op| {
                op.get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|p| p.starts_with("/state"))
            });
            if touches_state {
                Interest::Refresh(session_id.clone())
            } else {
                Interest::None
            }
        }
        SessionChange::SessionPatched {
            session_id, patch, ..
        } => {
            let moved = patch.iter().any(|op| {
                op.get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|p| p == "/cwd" || p.starts_with("/status"))
            });
            if moved {
                Interest::Refresh(session_id.clone())
            } else {
                Interest::None
            }
        }
        _ => Interest::None,
    }
}

/// Refreshes one session and sends `changesUpdated` when its changes differ from the last refresh.
pub(crate) async fn refresh_and_emit(shared: &Arc<Shared>, session_id: &str) {
    let Some(view) = shared.store.view(session_id) else {
        return;
    };
    let session_facts = facts(&view.state);
    let Some(folder) = session_facts.cwd.clone() else {
        return;
    };
    let hub_owner = shared.clone();
    let id = session_id.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let tracker = hub_owner.changes.ensure(&id, &folder, None);
        let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
        let before = guard.fingerprint.clone();
        let view = build_view(&hub_owner.changes, &id, &mut guard, &session_facts);
        (before != guard.fingerprint, view)
    })
    .await;
    match result {
        Ok((changed, view)) => emit_if_changed(shared, changed, &view),
        Err(error) => warn!(session_id, %error, "changes: refresh task failed"),
    }
}

/// Sends `changesUpdated` for `view` when `changed`. Every refresh reports its own change, whoever asked
/// for it (a client's read must not swallow the event other clients wait for).
pub(crate) fn emit_if_changed(shared: &Shared, changed: bool, view: &ChangesView) {
    if !changed {
        return;
    }
    shared.emit(CoreEvent::ChangesUpdated {
        session_id: view.session_id.clone(),
        mode: view.mode,
        files: u32::try_from(
            view.files.len() + view.turns.iter().map(|t| t.files.len()).sum::<usize>(),
        )
        .unwrap_or(u32::MAX),
        fingerprint: view.fingerprint.clone(),
    });
}

/// Blocking: refreshes `tracker` and builds the session's view (git or effects mode). Updates the tracker's
/// fingerprint, observed labels and last diff.
pub(crate) fn build_view(
    hub: &Hub,
    session_id: &str,
    tracker: &mut Tracker,
    session: &Facts,
) -> ChangesView {
    let started = Instant::now();
    let folder = tracker.folder.to_string_lossy().into_owned();
    let mut observed = std::mem::take(&mut tracker.observed);
    let mut last = None;
    let mut gone_baseline = None;
    let mut view = ChangesView {
        session_id: session_id.to_owned(),
        mode: ChangesMode::None,
        folder: Some(folder),
        repo_root: None,
        scope: None,
        reason: None,
        baseline_lost: false,
        baseline_at_ms: None,
        files: Vec::new(),
        fingerprint: String::new(),
        notes: Vec::new(),
        turns: Vec::new(),
        offer_history: false,
        watching: false,
        took_ms: 0,
    };
    match &tracker.review {
        Some(Review::Unavailable(reason)) => {
            view.mode = ChangesMode::Unavailable;
            view.reason = Some(reason.clone());
            view.fingerprint = format!("unavailable:{reason}");
        }
        Some(Review::Lost {
            reason,
            against_head,
        }) => {
            // Read only: shown against the last commit, nothing can be kept or reverted until the user
            // starts reviewing again.
            view.mode = ChangesMode::Git;
            view.baseline_lost = true;
            view.reason = Some(reason.clone());
            match against_head.as_deref().map(vw_changes::refresh) {
                Some(Ok(diff)) => {
                    view.repo_root = against_head
                        .as_ref()
                        .map(|b| b.repo().root.to_string_lossy().into_owned());
                    view.fingerprint = format!("lost:{}", diff.fingerprint);
                    view.notes.clone_from(&diff.notes);
                    view.files = diff.files;
                }
                Some(Err(error)) => view.fingerprint = format!("lost-failed:{error}"),
                None => view.fingerprint = "lost".into(),
            }
        }
        Some(Review::Git { baseline, watcher }) => {
            view.repo_root = Some(baseline.repo().root.to_string_lossy().into_owned());
            view.scope = Some(baseline.repo().scope.clone());
            view.baseline_at_ms = Some(baseline.state().taken_at_ms);
            view.watching = watcher.is_some();
            match vw_changes::refresh(baseline) {
                Ok(mut diff) => {
                    label_turns(&mut diff, &baseline.repo().root, &mut observed, session);
                    view.mode = ChangesMode::Git;
                    view.fingerprint.clone_from(&diff.fingerprint);
                    view.notes.clone_from(&diff.notes);
                    view.files.clone_from(&diff.files);
                    last = Some(diff);
                }
                Err(vw_changes::ChangesError::BaselineLost(reason)) => {
                    gone_baseline = Some((baseline.repo().clone(), baseline.dir().clone(), reason));
                }
                Err(error) => {
                    info!(session_id, %error, "changes: refresh failed");
                    view.mode = ChangesMode::Unavailable;
                    view.reason = Some(error.to_string());
                    view.fingerprint = format!("failed:{error}");
                }
            }
        }
        None => effects_view(hub, tracker, session, &mut view),
    }
    if last.is_some() {
        tracker.last = last;
    }
    tracker.observed = observed;
    if let Some((repo, dir, reason)) = gone_baseline {
        // The baseline's objects are gone for good: switch to the read-only view, never re-baseline.
        tracker.review = Some(lost_review(repo, &dir, reason));
        return build_view(hub, session_id, tracker, session);
    }
    view.took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracker.fingerprint = view.fingerprint.clone();
    view
}

/// Turn labels: a file effect of a turn names the file (`effect`), else the file first changed while a turn
/// ran (`observed`, kept while its blob is unchanged).
fn label_turns(
    diff: &mut Diff,
    root: &Path,
    observed: &mut HashMap<String, (Option<String>, TurnLabel)>,
    session: &Facts,
) {
    // Labels of files that are no longer changed are dropped (a later change is labelled afresh).
    observed.retain(|path, _| diff.files.iter().any(|f| &f.path == path));
    let by_path = last_turn_by_path(session);
    let running = session.running.as_deref().and_then(|id| session.turn(id));
    for file in &mut diff.files {
        // The same path value every consumer uses: git's bytes resolved once (vw-changes `inside_bytes`).
        let Ok(absolute) = vw_changes::inside_bytes(root, &file.raw_path) else {
            continue;
        };
        if let Some(turn) = by_path
            .get(&path_key(&absolute))
            .and_then(|id| session.turn(id))
        {
            file.turn = Some(TurnLabel {
                number: turn.number,
                turn_id: turn.id.clone(),
                source: TurnSource::Effect,
            });
            continue;
        }
        if let Some((_, label)) = observed.get(&file.path) {
            file.turn = Some(label.clone());
        } else if let Some(turn) = running {
            let label = TurnLabel {
                number: turn.number,
                turn_id: turn.id.clone(),
                source: TurnSource::Observed,
            };
            observed.insert(file.path.clone(), (file.blob.clone(), label.clone()));
            file.turn = Some(label);
        }
    }
}

/// Effects mode: files from Vibe's completed writes and edits, by turn; the version-history offer.
fn effects_view(hub: &Hub, tracker: &Tracker, session: &Facts, view: &mut ChangesView) {
    let base = session.cwd.as_deref();
    let mut turns: Vec<TurnChanges> = Vec::new();
    for touch in &session.touches {
        let Some(turn) = touch.turn_id.as_deref().and_then(|id| session.turn(id)) else {
            continue;
        };
        if turns.last().is_none_or(|t| t.turn_id != turn.id) {
            if let Some(existing) = turns.iter().position(|t| t.turn_id == turn.id) {
                let moved = turns.remove(existing);
                turns.push(moved);
            } else {
                turns.push(TurnChanges {
                    turn_id: turn.id.clone(),
                    number: turn.number,
                    anchor_entry_id: turn.anchor.clone(),
                    files: Vec::new(),
                });
            }
        }
        let Some(current) = turns.last_mut() else {
            continue;
        };
        let path = shown(&touch.path, base);
        match current.files.iter_mut().find(|f| f.path == path) {
            Some(file) => file.entry_ids.push(touch.entry_id.clone()),
            None => current.files.push(TurnFile {
                path,
                entry_ids: vec![touch.entry_id.clone()],
                created: touch.created,
            }),
        }
    }
    turns.sort_by_key(|t| t.number);
    if turns.is_empty() {
        view.mode = ChangesMode::None;
        view.fingerprint = "none".into();
        return;
    }
    view.mode = ChangesMode::Effects;
    let changed: Vec<String> = turns
        .iter()
        .flat_map(|t| t.files.iter().map(|f| f.path.clone()))
        .collect();
    let dismissed = hub
        .dir
        .as_deref()
        .is_some_and(|d| vw_changes::offer_dismissed(d, &tracker.folder));
    view.offer_history = !dismissed && vw_changes::looks_like_code(&tracker.folder, &changed);
    let ids: Vec<&[u8]> = session
        .touches
        .iter()
        .map(|t| t.entry_id.as_bytes())
        .collect();
    view.fingerprint = format!(
        "effects:{}:{}",
        vw_changes::fnv_hex(&ids),
        u8::from(view.offer_history)
    );
    view.turns = turns;
}
