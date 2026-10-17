//! `worktrees inspect` and `worktrees remove`:
//! a copy is removed only after the user saw exactly what goes with it, never with Vibe's default
//! parameters, and only a copy Vibe Work made for that session.
//!
//! Authorization: the copy is found from the session's record (our own state), never from a path a client
//! names or the session's folder; its folder (git's worktree list for the recorded branch) must resolve,
//! without a symlink, inside Vibe's worktree folder (`$VIBE_HOME/worktrees`).
//! Inspect = our own reading (vw-worktrees `inspect`: unsaved files incl. ignored ones that are not build
//! output, nested repositories, commits no other branch holds, in-progress states; git never reads the working
//! tree) + the preflight + Vibe's side effect free probe (`{cwd, inspect: true}`, only after the preflight
//! passed, because it runs git in the copy) for "not Vibe's", "gone", "held". Every
//! origin gets the lists.
//! Remove = the same inspection again, before and after confirmation; its fingerprint must equal the one the user saw (else `stale`); the
//! session's channel is closed so its own hold goes; then `{cwd, force: true, deleteBranch, inspect: false}`
//! behind the native dialog (the window; the Debug API acts as the user and is not asked) naming the folder, up to 8 files and the branch
//! outcome. `force: true` because the user saw the list (no hidden `refs/vibe/reaped` ref of untracked files that
//! a mirror push would publish); `deleteBranch` only for a branch Vibe Work made that holds no commit missing
//! from every other branch and remote.
//!
//! Depends on: vw-worktrees, `worktrees` (state, helpers), `dialog_texts`, `dispatch` (`forward`), the gate.
//! Called by: `worktrees`, `confirmer` (`describe_raw`).
//! Invariant: the only removal call this core makes states `force`, `deleteBranch` and `inspect: false`; a
//! client's own removal call is refused (`worktrees::raw_worktree_refusal`) and the gate refuses defaults.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vw_worktrees::{Decision, Inspection, Record};

use crate::command::{Origin, requester};
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::worktrees::{blocking, core_error, find_path};
use crate::worktrees_view::{BranchPlan, RemoveOutcome, WorktreeInspection};

const METHOD: &str = "workspace/git/worktrees/remove";
fn names(paths: &[String]) -> String {
    paths
        .iter()
        .map(|p| vw_gate::dialog_quote(p))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The dialog sentence for a removal nobody prepared (a raw `call`, which the core refuses for clients; kept
/// for defence in depth): from the params only.
pub(crate) fn describe_raw(params: &Value) -> String {
    let cwd = params.get("cwd").and_then(Value::as_str).unwrap_or("?");
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    let branch = params.get("deleteBranch").and_then(Value::as_bool) == Some(true);
    format!(
        "Remove the git worktree folder {}{}. Its branch is {}. The files it holds were not listed to you.",
        vw_gate::dialog_quote(cwd),
        if force {
            " and discard its uncommitted changes and new files without a copy"
        } else {
            ""
        },
        if branch { "deleted" } else { "kept" },
    )
}

/// The copy a request is about: always from the session's record.
struct Target {
    session_id: String,
    record: Record,
    /// The folder, checked to lie inside Vibe's worktree folder; `None` when git lists none for the branch.
    path: Option<PathBuf>,
    /// Why the folder may not be acted on (outside Vibe's folder, a symlink).
    misplaced: Option<String>,
}

/// Why `path` may not be removed: it must resolve, without a symlink, inside `root`.
pub(crate) fn placement(path: &Path, root: &Path) -> Option<String> {
    let link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    let inside = match (path.canonicalize(), root.canonicalize()) {
        (Ok(p), Ok(r)) => p.starts_with(&r) && p != r,
        _ => false,
    };
    (link || !inside).then(|| {
        "This copy's folder is not inside Vibe's worktree folder (or is reached through a link), so Vibe Work does not remove it.".to_owned()
    })
}

fn plan_branch(record: &Record, found: &Inspection) -> BranchPlan {
    if !record.created_branch {
        BranchPlan::Kept {
            reason: "Vibe Work did not create it".to_owned(),
        }
    } else if found.unmerged_total > 0 {
        BranchPlan::Kept {
            reason: format!(
                "it has {} commit(s) no other branch contains",
                found.unmerged_total
            ),
        }
    } else if found.branch.as_deref() != Some(record.branch.as_str()) {
        BranchPlan::Kept {
            reason: "the copy is on another branch now".to_owned(),
        }
    } else {
        BranchPlan::Deleted
    }
}

impl Core {
    fn worktree_target(&self, session_id: &str) -> Result<Target, CoreError> {
        let records = self.shared.worktrees.records()?;
        let record = records.get(session_id).ok_or_else(|| {
            CoreError::new(
                ErrorCode::NotFound,
                "Vibe Work made no copy for this session",
            )
        })?;
        let path = find_path(&record.repo_root, &record.branch);
        let misplaced = path
            .as_deref()
            .and_then(|p| placement(p, self.shared.worktrees.vibe_root()));
        Ok(Target {
            session_id: session_id.to_owned(),
            record,
            path,
            misplaced,
        })
    }

    /// Why the session's state forbids a removal now. The store learns `archivedAt` from Vibe's
    /// `session/updated`, which can arrive after the archive's answer, so a session the store does not show as
    /// archived is read from Vibe once more (`session/read`, straight through the gate, no store change) before
    /// it counts as still in use. For a session archived before its first turn, our own record remembers the
    /// user's decision because Vibe has no saved session to mark.
    async fn session_block(&self, target: &Target) -> Option<String> {
        if target.record.session_deleted {
            return None;
        }
        let id = target.session_id.as_str();
        let view = self.shared.store.view(id)?;
        let session = view.state.get("session")?;
        let status = session.pointer("/status/type").and_then(Value::as_str);
        if matches!(status, Some("running" | "blocked")) {
            return Some("Its session is still working. Stop it first.".to_owned());
        }
        let archived = |s: &Value| s.get("archivedAt").is_some_and(|v| !v.is_null());
        if archived(session)
            || (target.record.session_archived
                && view
                    .state
                    .get("turns")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty))
        {
            return None;
        }
        let read = self
            .shared
            .gate
            .call(
                "session/read",
                json!({"sessionId": id, "history": {"limit": 1}, "turns": {"limit": 1}}),
                None,
            )
            .await;
        let read = match read {
            Ok(read) => read,
            Err(error) => {
                return Some(format!(
                    "Its session could not be checked. Try again: {error}"
                ));
            }
        };
        let current = read.result.pointer("/state/session");
        if current
            .and_then(|s| s.pointer("/status/type"))
            .and_then(Value::as_str)
            .is_some_and(|status| matches!(status, "running" | "blocked"))
        {
            return Some("Its session is still working. Stop it first.".to_owned());
        }
        let archived_now = current.is_some_and(archived);
        (!archived_now).then(|| {
            "Its session still uses this copy. Archive or delete the session first.".to_owned()
        })
    }

    /// Vibe's probe: `(outcome, holders)`. Only called after the preflight passed.
    async fn vibe_probe(&self, origin: Origin, path: &str) -> Result<(String, u32), CoreError> {
        let reply = self
            .forward(origin, METHOD, json!({"cwd": path, "inspect": true}), None)
            .await?;
        let outcome = reply
            .result
            .get("outcome")
            .and_then(Value::as_str)
            .unwrap_or("kept_error")
            .to_owned();
        let holders = reply
            .result
            .get("holders")
            .and_then(Value::as_u64)
            .and_then(|h| u32::try_from(h).ok())
            .unwrap_or(0);
        Ok((outcome, holders))
    }

    /// The full inspection (lists included) and why a removal is blocked.
    async fn inspect_target(
        &self,
        origin: Origin,
        target: &Target,
    ) -> Result<(Inspection, Option<String>, Option<u32>), CoreError> {
        let scan = self.shared.worktrees.scan_dir()?;
        let (path, branch) = (target.path.clone(), target.record.branch.clone());
        let (found, risky) = blocking(move || -> Result<_, CoreError> {
            let Some(path) = path else {
                return Ok((Inspection::gone(), None));
            };
            let found = vw_worktrees::inspect(&path, Some(&branch), &scan).map_err(core_error)?;
            let risky = if found.exists {
                vw_worktrees::preflight(&path).map_err(core_error)?
            } else {
                None
            };
            Ok((found, risky))
        })
        .await??;
        let mut blocked = if !found.exists {
            Some("The copy's folder is gone; there is nothing to remove.".to_owned())
        } else if let Some(why) = &target.misplaced {
            Some(why.clone())
        } else if let Some(reason) = risky {
            Some(format!(
                "{reason} Remove this copy yourself (git worktree remove)."
            ))
        } else if !found.blockers.is_empty() {
            Some(found.blockers.join(" "))
        } else {
            self.session_block(target).await
        };
        let mut holders = None;
        if blocked.is_none()
            && let Some(path) = &target.path
        {
            let (outcome, held) = self.vibe_probe(origin, &path.display().to_string()).await?;
            holders = Some(held);
            let own_channel = self.shared.channels.has_channel(&target.session_id);
            blocked = match outcome.as_str() {
                "kept_unmanaged" => Some("Vibe did not make this folder (or no longer manages it), so Vibe Work does not remove it.".to_owned()),
                "not_found" => Some("The copy's folder is gone; there is nothing to remove.".to_owned()),
                // Its own session's hold goes when the channel closes (done before a removal).
                "kept_in_use" if !own_channel || held > 1 => Some(format!("{held} other session(s) use this copy.")),
                "kept_error" => Some("Vibe could not check this copy.".to_owned()),
                _ => None,
            };
        }
        Ok((found, blocked, holders))
    }

    /// The view of an inspection (the same for every origin).
    fn shape(
        target: &Target,
        found: Inspection,
        blocked: Option<String>,
        holders: Option<u32>,
    ) -> WorktreeInspection {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        WorktreeInspection {
            session_id: target.session_id.clone(),
            path: target.path.as_ref().map(|p| p.display().to_string()),
            exists: found.exists,
            branch: found
                .branch
                .clone()
                .or_else(|| Some(target.record.branch.clone())),
            base: Some(target.record.base_name.clone()),
            file_count: count(found.files.len()),
            unmerged_total: found.unmerged_total,
            build_output: found.build_output,
            stash_count: count(found.stashes.len()),
            branch_plan: plan_branch(&target.record, &found),
            blocked,
            holders,
            fingerprint: found.fingerprint.clone(),
            files: found.files,
            unmerged: found.unmerged,
            stashes: found.stashes,
        }
    }

    /// `worktrees inspect`.
    ///
    /// # Errors
    /// `notFound` (no copy made here), `internal` (git failures: fail closed), Vibe's probe errors.
    pub(crate) async fn worktree_inspect(
        &self,
        origin: Origin,
        session_id: &str,
    ) -> Result<WorktreeInspection, CoreError> {
        let target = self.worktree_target(session_id)?;
        let (found, blocked, holders) = self.inspect_target(origin, &target).await?;
        Ok(Self::shape(&target, found, blocked, holders))
    }

    /// `worktrees remove`.
    ///
    /// # Errors
    /// `denied` (blocked, see the inspection), `stale` (the state differs from `fingerprint`),
    /// `confirmationDeclined`.
    pub(crate) async fn worktree_remove(
        &self,
        origin: Origin,
        session_id: &str,
        fingerprint: &str,
    ) -> Result<RemoveOutcome, CoreError> {
        let _one_at_a_time = self.shared.worktrees.lock.lock().await;
        let target = self.worktree_target(session_id)?;
        let (found, blocked, _) = self.inspect_target(origin, &target).await?;
        if let Some(reason) = blocked {
            return Err(CoreError::new(ErrorCode::Denied, reason));
        }
        if found.fingerprint != fingerprint {
            return Err(CoreError::new(
                ErrorCode::Stale,
                "The copy changed since you looked: check it again before removing it.",
            ));
        }
        let Some(path) = target.path.clone() else {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "the copy's folder is gone",
            ));
        };
        let path_text = path.display().to_string();
        self.shared.channels.close(&target.session_id).await;
        let (outcome, held) = self.vibe_probe(origin, &path_text).await?;
        if outcome == "kept_in_use" {
            return Err(CoreError::new(
                ErrorCode::Denied,
                format!("{held} other session(s) use this copy; it was not removed."),
            ));
        }
        let plan = plan_branch(&target.record, &found);
        let delete_branch = plan == BranchPlan::Deleted;
        let params = json!({"cwd": path_text, "force": true, "deleteBranch": delete_branch, "inspect": false});
        self.shared
            .dialog_texts
            .set(METHOD, &params, removal_sentence(&path_text, &found, &plan));
        let token = self
            .shared
            .gate
            .request_confirmation_as(METHOD, &params, requester(origin))
            .await
            .map_err(|e| {
                let _ = self.shared.dialog_texts.take(METHOD, &params);
                CoreError::from_gate(METHOD, &e)
            })?;
        // Consent may stay open while another process edits the copy. Re-resolve its placement and
        // repeat the full inspection before sending the destructive call.
        let current = self.worktree_target(session_id)?;
        let (after, blocked, _) = self.inspect_target(origin, &current).await?;
        if current.path != target.path || after.fingerprint != fingerprint {
            return Err(CoreError::new(
                ErrorCode::Stale,
                "The copy changed since you looked: check it again before removing it.",
            ));
        }
        if let Some(reason) = blocked {
            return Err(CoreError::new(ErrorCode::Denied, reason));
        }
        let reply = self
            .forward(origin, METHOD, params, Some(token.as_str().to_owned()))
            .await?;
        let text = |key: &str| {
            reply
                .result
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let vibe_outcome = text("outcome");
        let removed = vibe_outcome == "removed" && !path.exists();
        if removed {
            let records = self.shared.worktrees.records()?;
            records
                .update(&target.session_id, |r| r.decision = Some(Decision::Removed))
                .map_err(core_error)?;
            self.shared
                .emit(crate::event::CoreEvent::WorktreesChanged {});
        }
        Ok(RemoveOutcome {
            removed,
            branch_deleted: reply.result.get("branchDeleted").and_then(Value::as_bool)
                == Some(true),
            vibe_outcome,
            reasons: reply
                .result
                .get("reasons")
                .and_then(Value::as_array)
                .map(|r| {
                    r.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// The native dialog's sentence for a prepared removal: the folder, what is discarded, the branch outcome.
fn removal_sentence(path: &str, found: &Inspection, plan: &BranchPlan) -> String {
    let branch = found.branch.as_deref().unwrap_or("(detached)");
    let mut text = format!(
        "Remove the copy {} (branch {}).",
        vw_gate::dialog_quote(path),
        vw_gate::dialog_quote(branch),
    );
    let paths: Vec<String> = found.files.iter().map(|f| f.path.clone()).collect();
    if paths.is_empty() {
        text.push_str(" It has no uncommitted changes and no other unsaved files.");
    } else {
        let _ = write!(
            text,
            " The unsaved state of {} file(s) is discarded for good: {}.",
            paths.len(),
            names(&paths)
        );
    }
    if found.build_output > 0 {
        let _ = write!(
            text,
            " {} build output item(s) go with the folder.",
            found.build_output
        );
    }
    match plan {
        BranchPlan::Deleted => {
            text.push_str(" The branch is deleted too (every commit on it is in another branch).");
        }
        BranchPlan::Kept { reason } => {
            let _ = write!(
                text,
                " The branch stays ({}).",
                vw_gate::dialog_line(reason)
            );
        }
    }
    text
}
