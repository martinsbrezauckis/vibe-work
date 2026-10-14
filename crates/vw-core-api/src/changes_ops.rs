//! The `changes` command: read, keep, revert, undo preview, folder sessions, version history; and
//! the hook that takes a session's baseline before its first piece of work reaches Vibe.
//!
//! Depends on: `changes` (hub, trackers, views), `changes_facts`, `changes_view`, `core::Core`, `dispatch`
//! (`forward` for `session/rewind/read`), vw-changes. Called by: `dispatch`.
//! Invariants:
//! - Keep and revert act only on what the client saw (blob id, hunk id or fingerprint, checked by
//!   vw-changes); a revert deletes only files the request names.
//! - `enableHistory` writes git state into the user's folder, so it needs `confirmed: true` (the window's
//!   confirmation, built from `planHistory`; a Debug API client states it) and refuses a folder
//!   that is already a repository.
//! - Every origin may keep, revert and restart directly (removed the Debug API dialog).
//! - The undo of a turn itself is the existing `rewindSession` command (`restoreFiles`, `inplace`); this
//!   module only previews it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use serde_json::{Value, json};
use tracing::warn;

use crate::changes::{Review, Tracker, build_view, emit_if_changed};
use crate::changes_facts::{Facts, facts, path_key, shown};
use crate::changes_slots::Claim;
use crate::changes_view::{ChangesRequest, ChangesResult, FolderSession, UndoPreview, core_error};
use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};

impl Core {
    fn refuse_history_over_nested_repo(&self, session_id: &str) -> Result<(), CoreError> {
        if self
            .git_repos(session_id)?
            .iter()
            .any(|(repo, _)| repo.root != ".")
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "Nested repository in use",
            ));
        }
        Ok(())
    }

    /// Executes one Changes request for `origin`.
    ///
    /// # Errors
    /// `notFound` (session not loaded, turn unknown), `stale`, `confirmDeletes`, `denied` (an action that does
    /// not apply), `invalidParams`, `internal`
    /// (git failures, Changes off).
    pub async fn changes(
        &self,
        origin: Origin,
        request: ChangesRequest,
    ) -> Result<ChangesResult, CoreError> {
        match request {
            ChangesRequest::Read { session_id } => self.changes_read(&session_id).await,
            ChangesRequest::Keep { session_id, target } => {
                self.changes_act(&session_id, move |tracker| {
                    let Some(Review::Git { baseline, .. }) = &mut tracker.review else {
                        return Err(no_review(tracker));
                    };
                    vw_changes::keep(baseline, &target)
                        .map(|_| ())
                        .map_err(core_error)
                })
                .await
            }
            ChangesRequest::Revert {
                session_id,
                target,
                delete_paths,
            } => {
                let deletes = delete_paths.unwrap_or_default();
                self.changes_act(&session_id, move |tracker| {
                    let Some(Review::Git { baseline, .. }) = &tracker.review else {
                        return Err(no_review(tracker));
                    };
                    vw_changes::revert(baseline, &target, &deletes)
                        .map(|_| ())
                        .map_err(core_error)
                })
                .await
            }
            ChangesRequest::PreviewUndo {
                session_id,
                turn_id,
            } => self.undo_preview(origin, &session_id, &turn_id).await,
            ChangesRequest::FolderSessions { cwd } => self.folder_sessions(&cwd).await,
            ChangesRequest::PlanHistory { session_id } => {
                self.refuse_history_over_nested_repo(&session_id)?;
                let folder = self.plain_folder(&session_id)?;
                let plan =
                    blocking(move || vw_changes::plan_init(&folder).map_err(core_error)).await??;
                Ok(ChangesResult::InitPlan { plan })
            }
            ChangesRequest::EnableHistory {
                session_id,
                confirmed,
            } => {
                // `confirmed: true` is the consent, from the window's own confirmation or from a Debug API client,
                // which acts as the user.
                if !confirmed {
                    return Err(CoreError::new(
                        ErrorCode::InvalidParams,
                        "enableHistory needs the user's confirmation (confirmed: true)",
                    ));
                }
                self.refuse_history_over_nested_repo(&session_id)?;
                let folder = self.plain_folder(&session_id)?;
                let claim = self.changes_claim(&session_id).await?;
                let hub_owner = self.shared.clone();
                let id = session_id.clone();
                blocking(move || {
                    vw_changes::init_repository(&folder).map_err(core_error)?;
                    hub_owner.changes.forget(&id, false);
                    hub_owner.changes.ensure(&id, &folder, Some(&claim));
                    Ok::<(), CoreError>(())
                })
                .await??;
                self.changes_read(&session_id).await
            }
            ChangesRequest::Restart { session_id } => {
                let folder = self.session_facts(&session_id)?.1;
                // Waits for a baseline still being taken (also one past its deadline): two writers never
                // share a session's baseline folder, and the restart's baseline is never dropped by it.
                let claim = self.changes_claim(&session_id).await?;
                let hub_owner = self.shared.clone();
                let id = session_id.clone();
                blocking(move || {
                    hub_owner.changes.restart(&id, &folder, &claim);
                })
                .await?;
                self.changes_read(&session_id).await
            }
            ChangesRequest::DismissOffer { session_id } => {
                let folder = self.session_facts(&session_id)?.1;
                let dir = self.shared.changes.dir.clone().ok_or_else(changes_off)?;
                blocking(move || vw_changes::dismiss_offer(&dir, &folder).map_err(core_error))
                    .await??;
                self.changes_read(&session_id).await
            }
        }
    }

    /// Takes the session's baseline if it has none, before `method` reaches Vibe (`dispatch::forward`).
    /// Bounded by `CoreConfig::changes_baseline_limit`; never fails the call.
    ///
    /// Past the limit the review is "unavailable" at once (a new generation), and the work goes on without a
    /// baseline. The blocking task cannot be stopped: a cleanup task waits for it (its own late result is
    /// dropped by the generation check in `Hub::ensure`), removes what it left in the session's baseline
    /// folder, and only then releases the claim, so "Start reviewing from now" never overlaps it.
    pub(crate) async fn changes_before_work(&self, params: &Value) {
        if self.shared.changes.dir.is_none() {
            return;
        }
        let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let Ok((session, folder)) = self.session_facts(session_id) else {
            return;
        };
        drop(session);
        if self.review_ready(session_id, &folder) {
            return;
        }
        let deadline = tokio::time::Instant::now() + self.shared.config.changes_baseline_limit;
        // One writer of a session's baseline folder at a time: wait (within the limit) for one running.
        let Some(claim) = self
            .shared
            .changes
            .claim_wait(session_id, Some(deadline))
            .await
        else {
            warn!(
                session_id,
                "changes: the session's baseline is still being written; the work starts without it"
            );
            return;
        };
        if self.review_ready(session_id, &folder) {
            return;
        }
        let hub_owner = self.shared.clone();
        let id = session_id.to_owned();
        let target = folder.clone();
        let hooks = self.shared.config.changes_test_hooks.clone();
        let mut work = tokio::task::spawn_blocking(move || {
            if let Some(hooks) = &hooks {
                hooks.before_take(&id);
            }
            hub_owner.changes.ensure(&id, &target, Some(&claim));
            if let Some(hooks) = &hooks {
                hooks.after_take(&id);
            }
            claim
        });
        if tokio::time::timeout_at(deadline, &mut work).await.is_ok() {
            return;
        }
        warn!(
            session_id,
            "changes: baseline took too long; the review is off for this session"
        );
        // Late, the baseline could include the agent's first edits: "unavailable" now, in a new generation.
        let generation = self.shared.changes.expire(
            session_id,
            &folder,
            "The folder was too big to record before the session started.",
        );
        let hub_owner = self.shared.clone();
        let id = session_id.to_owned();
        tokio::spawn(async move {
            let claim = work.await;
            let owner = hub_owner.clone();
            let _ =
                tokio::task::spawn_blocking(move || owner.changes.remove_data_if(&id, generation))
                    .await;
            drop(claim);
        });
    }

    /// Whether the session's tracker for `folder` already has its review (never waits for a tracker that is
    /// busy: then it is not known to be ready).
    fn review_ready(&self, session_id: &str, folder: &Path) -> bool {
        let Some(tracker) = self.shared.changes.tracker(session_id) else {
            return false;
        };
        let guard = match tracker.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        guard.opened && guard.folder == folder && guard.review.is_some()
    }

    /// The session's claim on its baseline folder, waiting for any holder (`changes_slots`).
    async fn changes_claim(&self, session_id: &str) -> Result<Claim, CoreError> {
        self.shared
            .changes
            .claim_wait(session_id, None)
            .await
            .ok_or_else(|| CoreError::new(ErrorCode::Internal, "the review is busy"))
    }

    /// The session's facts and folder.
    fn session_facts(&self, session_id: &str) -> Result<(Facts, PathBuf), CoreError> {
        let view = self
            .shared
            .store
            .view(session_id)
            .ok_or_else(|| CoreError::not_loaded(session_id))?;
        let session = facts(&view.state);
        let folder = session
            .cwd
            .clone()
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "the session has no folder yet"))?;
        Ok((session, folder))
    }

    /// The folder of a session that is not in a git repository (version-history requests).
    fn plain_folder(&self, session_id: &str) -> Result<PathBuf, CoreError> {
        let folder = self.session_facts(session_id)?.1;
        if let Some(tracker) = self.shared.changes.tracker(session_id)
            && tracker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .review
                .is_some()
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "this folder already has version history",
            ));
        }
        Ok(folder)
    }

    pub(crate) async fn changes_read(&self, session_id: &str) -> Result<ChangesResult, CoreError> {
        if self.shared.changes.dir.is_none() {
            return Err(changes_off());
        }
        let (session, folder) = self.session_facts(session_id)?;
        let hub_owner = self.shared.clone();
        let id = session_id.to_owned();
        let view = blocking(move || {
            let tracker = hub_owner.changes.ensure(&id, &folder, None);
            let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
            let before = guard.fingerprint.clone();
            let view = build_view(&hub_owner.changes, &id, &mut guard, &session);
            (before != guard.fingerprint, view)
        })
        .await?;
        emit_if_changed(&self.shared, view.0, &view.1);
        Ok(ChangesResult::View { view: view.1 })
    }

    /// Runs a keep/revert on the session's tracker, then answers with the fresh view.
    async fn changes_act(
        &self,
        session_id: &str,
        act: impl FnOnce(&mut Tracker) -> Result<(), CoreError> + Send + 'static,
    ) -> Result<ChangesResult, CoreError> {
        let (session, folder) = self.session_facts(session_id)?;
        let hub_owner = self.shared.clone();
        let id = session_id.to_owned();
        let view = blocking(move || {
            let tracker: Arc<Mutex<Tracker>> = hub_owner.changes.ensure(&id, &folder, None);
            let mut guard = tracker.lock().unwrap_or_else(PoisonError::into_inner);
            act(&mut guard)?;
            let before = guard.fingerprint.clone();
            let view = build_view(&hub_owner.changes, &id, &mut guard, &session);
            Ok::<_, CoreError>((before != guard.fingerprint, view))
        })
        .await??;
        emit_if_changed(&self.shared, view.0, &view.1);
        Ok(ChangesResult::View { view: view.1 })
    }

    async fn undo_preview(
        &self,
        origin: Origin,
        session_id: &str,
        turn_id: &str,
    ) -> Result<ChangesResult, CoreError> {
        let (session, folder) = self.session_facts(session_id)?;
        let turn = session
            .turn(turn_id)
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, format!("no turn {turn_id:?}")))?
            .clone();
        let anchor = turn.anchor.clone().ok_or_else(|| {
            CoreError::new(
                ErrorCode::NotFound,
                "this turn has no user message to rewind to",
            )
        })?;
        let reply = self
            .forward(
                origin,
                "session/rewind/read",
                json!({"sessionId": session_id, "entryId": anchor}),
                None,
            )
            .await?;
        // Vibe resolves the paths (`_unified_harness_snapshots.py` `_resolve`): absolute.
        let restorable: Vec<PathBuf> = reply
            .result
            .get("paths")
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .filter_map(Value::as_str)
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        let later: HashSet<&str> = session
            .turns
            .iter()
            .filter(|t| t.number >= turn.number)
            .map(|t| t.id.as_str())
            .collect();
        let restorable_set: HashSet<PathBuf> = restorable.iter().map(|p| path_key(p)).collect();
        let mut not_restorable: Vec<String> = Vec::new();
        for touch in &session.touches {
            let in_scope = touch.turn_id.as_deref().is_some_and(|t| later.contains(t));
            let shown_path = shown(&touch.path, Some(&folder));
            if in_scope
                && !restorable_set.contains(&path_key(&touch.path))
                && !not_restorable.contains(&shown_path)
            {
                not_restorable.push(shown_path);
            }
        }
        Ok(ChangesResult::UndoPreview {
            preview: UndoPreview {
                turn_id: turn.id.clone(),
                anchor_entry_id: anchor,
                restorable: restorable.iter().map(|p| shown(p, Some(&folder))).collect(),
                not_restorable,
                turns_removed: u32::try_from(later.len()).unwrap_or(u32::MAX),
            },
        })
    }

    async fn folder_sessions(&self, cwd: &str) -> Result<ChangesResult, CoreError> {
        let folder = PathBuf::from(cwd);
        let probe = folder.clone();
        let repo = blocking(move || vw_changes::discover(&probe).ok().flatten()).await?;
        let root = repo
            .as_ref()
            .map_or_else(|| folder.clone(), |r| r.root.clone());
        let mut sessions = Vec::new();
        for view in self.shared.store.views() {
            let session = facts(&view.state);
            let Some(their) = session.cwd.as_deref() else {
                continue;
            };
            // Compared as `path_key`s: git's `C:/x` and the session's `\\?\C:\x` are one folder on Windows.
            let their = path_key(their);
            if !(their == path_key(&folder) || their.starts_with(path_key(&root))) {
                continue;
            }
            let changed = self
                .shared
                .changes
                .tracker(&view.session_id)
                .and_then(|t| {
                    t.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .last
                        .as_ref()
                        .map(|d| d.files.len())
                })
                .unwrap_or(0);
            let running = session.running.is_some();
            if running || changed > 0 {
                sessions.push(FolderSession {
                    session_id: view.session_id.clone(),
                    title: session.title.clone(),
                    running,
                    changed_files: u32::try_from(changed).unwrap_or(u32::MAX),
                });
            }
        }
        Ok(ChangesResult::Folder {
            is_git: repo.is_some(),
            sessions,
        })
    }
}

fn no_review(tracker: &Tracker) -> CoreError {
    if matches!(tracker.review, Some(Review::Lost { .. })) {
        return CoreError::new(
            ErrorCode::Denied,
            "the starting point for this session was lost: start reviewing from now before keeping or reverting",
        );
    }
    CoreError::new(
        ErrorCode::Denied,
        "no git review for this session (the folder has no version history, or the review is unavailable)",
    )
}

fn changes_off() -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        "the Changes view is off (no data folder)",
    )
}

/// Runs blocking work off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("changes task failed: {e}")))
}
