//! Refreshes Changes views from file/effect events and attributes file edits to turns.
//! Depends on tracker state and git/effect facts; called by the Changes hub and command handlers.
use super::{Hub, Review, Tracker, emit_if_changed, lost_review};
use crate::changes_children::{ancestors, include_children};
use crate::changes_effects::effects_view;
use crate::changes_facts::{Facts, facts, last_turn_by_path, path_key};
use crate::changes_view::{ChangesMode, ChangesView};
use crate::core::Shared;
use crate::event::{CoreEvent, SessionChange};
use crate::subscription::CoreSubscription;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, PoisonError, Weak},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{info, warn};
use vw_changes::{Diff, TurnLabel, TurnSource};

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
                        .map(|b| crate::changes_view::display_root(&b.repo().root));
                    view.fingerprint = format!("lost:{}", diff.fingerprint);
                    view.notes.clone_from(&diff.notes);
                    view.files = diff.files;
                }
                Some(Err(error)) => view.fingerprint = format!("lost-failed:{error}"),
                None => view.fingerprint = "lost".into(),
            }
        }
        Some(Review::Git { baseline, watcher }) => {
            view.repo_root = Some(crate::changes_view::display_root(&baseline.repo().root));
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
    if view.mode == ChangesMode::Git {
        view.turns =
            crate::changes_effects::child_turns(session, view.repo_root.as_deref().map(Path::new));
    }
    view.took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    // Child history may arrive after the file watcher. Publish newly known authors even if the diff is unchanged;
    // the public fingerprint remains Git's own so Keep/Revert admission uses the original diff identity.
    let authors = serde_json::to_vec(&view.turns).unwrap_or_default();
    tracker.fingerprint = format!("{}:{}", view.fingerprint, vw_changes::fnv_hex(&[&authors]));
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
    // Effect paths and git's paths are matched in one canonical form: the same folder can be written several ways
    // (git's root has links resolved, the agent writes the session folder as the session named it).
    let real_root = root.canonicalize().ok();
    let real_cwd = session
        .cwd
        .as_deref()
        .and_then(|cwd| cwd.canonicalize().ok());
    let by_path = last_turn_by_path(session, real_cwd.as_deref());
    let running = session.running.as_deref().and_then(|id| session.turn(id));
    for file in &mut diff.files {
        // The same path value every consumer uses: git's bytes resolved once (vw-changes `inside_bytes`).
        let Ok(absolute) = vw_changes::inside_bytes(root, &file.raw_path) else {
            continue;
        };
        let effect = by_path.get(&path_key(&absolute)).or_else(|| {
            let real = vw_changes::inside_bytes(real_root.as_deref()?, &file.raw_path).ok()?;
            by_path.get(&path_key(&real))
        });
        if let Some(turn) = effect.and_then(|id| session.turn(id)) {
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

/// Delay that merges a burst of effect events into one refresh.
const EFFECT_SETTLE: Duration = Duration::from_millis(150);

/// The refresh loop: effect and folder events (settled) and watcher signals -> refresh -> event.
pub(super) async fn run(
    weak: Weak<Shared>,
    mut subscription: CoreSubscription,
    mut signals: mpsc::UnboundedReceiver<String>,
    stop: Arc<tokio::sync::Notify>,
) {
    let mut due: HashMap<String, Instant> = HashMap::new();
    loop {
        let next = due.values().min().copied();
        tokio::select! {
            biased;
            () = stop.notified() => break,
            event = subscription.next() => {
                let Some(event) = event else { break };
                let Some(shared) = weak.upgrade() else { break };
                match interesting(&event) {
                    Interest::Refresh(id) => {
                        for target in ancestors(&shared.store, &id).into_iter().chain([id]) {
                            due.entry(target).or_insert_with(|| Instant::now() + EFFECT_SETTLE);
                        }
                    }
                    Interest::Snapshot(id) => {
                        let mut targets = ancestors(&shared.store, &id);
                        if shared.changes.tracker(&id).is_some() {
                            targets.push(id);
                        }
                        for target in targets {
                            due.entry(target).or_insert_with(|| Instant::now() + EFFECT_SETTLE);
                        }
                    }
                    Interest::Removed(id) => {
                        for target in ancestors(&shared.store, &id) {
                            due.entry(target).or_insert_with(|| Instant::now() + EFFECT_SETTLE);
                        }
                        shared.changes.forget(&id, false);
                    }
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
    Snapshot(String),
    Removed(String),
    None,
}

/// Which session an event should refresh: completed effects (a write, an edit, a command) and folder moves.
fn interesting(event: &CoreEvent) -> Interest {
    if let CoreEvent::Snapshot { snapshot } = event {
        return Interest::Snapshot(snapshot.session_id.clone());
    }
    let CoreEvent::Change { change } = event else {
        return Interest::None;
    };
    match change {
        SessionChange::Removed { session_id } => Interest::Removed(session_id.clone()),
        SessionChange::ChildUpserted { session_id, .. }
        | SessionChange::TurnUpserted { session_id, .. } => Interest::Refresh(session_id.clone()),
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
    let mut session_facts = facts(&view.state);
    let Some(folder) = session_facts.cwd.clone() else {
        return;
    };
    include_children(shared, session_id, &mut session_facts).await;
    let hub_owner = shared.clone();
    let id = session_id.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        if let Some(hooks) = &hub_owner.changes.hooks {
            hooks.before_refresh(&id);
        }
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
