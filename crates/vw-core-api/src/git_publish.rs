//! The `git` command's outward half: the
//! dispatcher for Push, pull requests (their logic in `git_pr.rs`), the PR card, prepared messages and "Bring
//! into main".
//!
//! Every action that sends something off the machine (a push, a PR made with gh) or moves the user's base branch
//! asks the native confirmation first, naming exactly what happens; a Debug API request is the user's consent
//! (the core's confirmer answers it without a dialog).
//!
//! Depends on: vw-git, `git_cmd` (place, errors), `git_pr`. Called by: `git_cmd::Core::git`.
//! Invariant: no network without a user's action.

use std::path::Path;

use serde_json::json;
use vw_git::merge::{self, BringKind};
use vw_git::push::{self, PushPlan};
use vw_git::state::{self, BranchState, RemoteInfo};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::{Place, git_error};
use crate::git_view::{BringPlanView, GitLocation, GitRequest, GitResult, PushPlanView};
use crate::worktrees::blocking;

pub(crate) fn requester(origin: Origin) -> vw_gate::Requester {
    if origin == Origin::DebugApi {
        vw_gate::Requester::DebugApi
    } else {
        vw_gate::Requester::Window
    }
}

pub(crate) fn folder_label(place: &Place) -> String {
    vw_gate::dialog_quote(&place.root.file_name().unwrap_or_default().to_string_lossy())
}

/// Full, sanitized push target for a native confirmation (the plan's `target` can hold credentials).
pub(crate) fn push_consent(plan: &PushPlan) -> String {
    let commits = match plan.commits {
        0 => "no new commits".to_owned(),
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    };
    let lfs = if plan.lfs {
        " Git LFS objects are not uploaded by this push."
    } else {
        ""
    };
    format!(
        "Push branch {} ({commits}) to remote {} at {} as {}? Commit: {}.{lfs}",
        vw_gate::dialog_quote(&plan.branch),
        vw_gate::dialog_quote(&plan.remote),
        vw_gate::dialog_quote(&plan.url),
        vw_gate::dialog_quote(&plan.remote_branch),
        vw_gate::dialog_quote(&plan.commit),
    )
}

impl Core {
    pub(crate) async fn git_publish(
        &self,
        origin: Origin,
        request: GitRequest,
    ) -> Result<GitResult, CoreError> {
        match request {
            GitRequest::PushPlan {
                session_id,
                repo,
                remote,
            } => {
                let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                let (state, plan) = self.push_plan(&place, remote).await?;
                let sentence = format!(
                    "In folder {}: {}",
                    folder_label(&place),
                    push_consent(&plan)
                );
                Ok(GitResult::PushPlan {
                    plan: PushPlanView {
                        plan,
                        sentence,
                        remotes: pushable(&state),
                    },
                })
            }
            GitRequest::Push {
                session_id,
                repo,
                remote,
                commit,
            } => {
                self.push_now(origin, &session_id, repo.as_deref(), remote, &commit)
                    .await
            }
            GitRequest::PrPlan {
                session_id,
                repo,
                remote,
            } => {
                let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                Ok(GitResult::PrPlan {
                    plan: self.pr_plan(&session_id, &place, remote).await?,
                })
            }
            request @ GitRequest::CreatePr { .. } => self.create_pr_request(origin, request).await,
            GitRequest::PrStatus {
                session_id,
                repo,
                refresh,
            } => {
                let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                Ok(GitResult::PrStatus {
                    status: self.pr_status(&session_id, &place, refresh).await?,
                })
            }
            GitRequest::PrepareMessage {
                session_id,
                repo,
                what,
            } => {
                let (selected, place) = self.git_selected_place(&session_id, repo.as_deref())?;
                if selected != "." && what == crate::git_view::PreparedKind::UpdateFromBase {
                    return Err(CoreError::new(
                        ErrorCode::Denied,
                        "Update from base uses the session repository",
                    ));
                }
                Ok(GitResult::Prepared {
                    text: self.prepared(&session_id, &place, what).await?,
                })
            }
            GitRequest::BringPlan { session_id, repo } => {
                self.git_bring_repo(session_id.as_str(), repo.as_deref())?;
                let place = self.git_place(&session_id)?;
                Ok(GitResult::BringPlan {
                    plan: self.bring_plan(&place).await?,
                })
            }
            GitRequest::BringIn {
                session_id,
                repo,
                base_commit,
                branch_commit,
            } => {
                self.git_bring_repo(session_id.as_str(), repo.as_deref())?;
                self.bring_in(origin, &session_id, &base_commit, &branch_commit)
                    .await
            }
            other => Err(CoreError::new(
                ErrorCode::Internal,
                format!("git {other:?} has no handler"),
            )),
        }
    }

    async fn push_now(
        &self,
        origin: Origin,
        session_id: &str,
        repo: Option<&str>,
        remote: String,
        commit: &str,
    ) -> Result<GitResult, CoreError> {
        let (_, place) = self.git_selected_place(session_id, repo)?;
        let (_, plan) = self.push_plan(&place, Some(remote)).await?;
        stale_unless(&plan.commit, commit)?;
        let sentence = format!(
            "In folder {}: {}",
            folder_label(&place),
            push_consent(&plan)
        );
        let params = json!({"sessionId": session_id, "root": place.root, "plan": plan});
        self.ask_native(requester(origin), "vibe-work/gitPush", params, sentence)
            .await?;
        self.git_validate_place(session_id, &place)?;
        let message = self.run_push(&place, plan).await?;
        Ok(GitResult::Pushed { message })
    }

    async fn bring_in(
        &self,
        origin: Origin,
        session_id: &str,
        base_commit: &str,
        branch_commit: &str,
    ) -> Result<GitResult, CoreError> {
        let place = self.git_place(session_id)?;
        let view = self.bring_plan(&place).await?;
        stale_unless(&view.plan.base_commit, base_commit)?;
        stale_unless(&view.plan.branch_commit, branch_commit)?;
        if view.plan.kind == BringKind::Conflicts {
            return Err(git_error(vw_git::GitError::Blocked(view.sentence)));
        }
        let params =
            json!({"sessionId": session_id, "base": view.plan.base, "branch": view.plan.branch});
        self.ask_native(
            requester(origin),
            "vibe-work/gitBringIn",
            params,
            view.sentence.clone(),
        )
        .await?;
        let _one = self.shared.git.lock.lock().await;
        let scratch = self.shared.git.folder("tmp")?;
        let main = place.main_root.clone();
        let plan = view.plan.clone();
        let commit = blocking(move || merge::apply(&main, &plan, &scratch))
            .await?
            .map_err(git_error)?;
        tracing::info!(session = session_id, "git bring in");
        Ok(GitResult::Brought { commit })
    }

    async fn push_plan(
        &self,
        place: &Place,
        remote: Option<String>,
    ) -> Result<(BranchState, PushPlan), CoreError> {
        let p = place.clone();
        blocking(move || {
            let state = state::read_state(&p.root, &p.git_dir, p.base.as_deref())?;
            if let Some(why) = state
                .in_progress
                .map(|op| format!("git has {} in progress here.", op.words()))
            {
                return Err(vw_git::GitError::Blocked(why));
            }
            let plan = push::plan(&p.root, &state, remote.as_deref())?;
            Ok((state, plan))
        })
        .await?
        .map_err(git_error)
    }

    pub(crate) async fn run_push(
        &self,
        place: &Place,
        plan: PushPlan,
    ) -> Result<String, CoreError> {
        let _one = self.shared.git.lock.lock().await;
        let root = place.root.clone();
        let done = blocking(move || push::push(&root, &plan))
            .await?
            .map_err(git_error)?;
        tracing::info!("git push done");
        Ok(done)
    }

    pub(crate) async fn bring_plan(&self, place: &Place) -> Result<BringPlanView, CoreError> {
        if place.location != GitLocation::Copy {
            return Err(git_error(vw_git::GitError::Blocked(
                "Bring into main is for sessions that work in their own copy.".to_owned(),
            )));
        }
        let (Some(base), Some(branch)) = (place.base.clone(), place.own_branch.clone()) else {
            return Err(git_error(vw_git::GitError::Blocked(
                "This copy started from a commit, not a branch.".to_owned(),
            )));
        };
        let main = place.main_root.clone();
        let plan = blocking(move || merge::plan(&main, &base, &branch))
            .await?
            .map_err(git_error)?;
        let sentence = bring_sentence(&plan, &place.main_root);
        Ok(BringPlanView { plan, sentence })
    }
}

fn bring_sentence(plan: &vw_git::merge::BringPlan, folder: &Path) -> String {
    let base = vw_gate::dialog_quote(&plan.base);
    let base = &base[1..base.len() - 1];
    let branch = vw_gate::dialog_quote(&plan.branch);
    let folder = vw_gate::dialog_quote(&folder.display().to_string());
    let folder = &folder[1..folder.len() - 1];
    let sentence = format!(
        "Bring the work from the copy {branch} into {base} in {folder}?\nIf you have unsaved changes there that would be overwritten, nothing is changed."
    );
    match plan.kind {
        BringKind::UpToDate => format!("{sentence}\n{base} already has every commit of {branch}."),
        BringKind::Conflicts => format!(
            "{sentence}\n{base} and {branch} change the same lines in {}. Nothing was changed. Ask Vibe to update the copy from {base} first.",
            plan.conflicts
                .iter()
                .map(|file| vw_gate::dialog_quote(file))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ if plan.checked_out_elsewhere || plan.checked_out.is_none() => {
            format!("{sentence}\n{}.", where_(plan))
        }
        _ => sentence,
    }
}

fn where_(plan: &vw_git::merge::BringPlan) -> String {
    match (&plan.checked_out, plan.checked_out_elsewhere) {
        (Some(path), false) => format!(
            "your folder {} is updated; git stops if your uncommitted files would be overwritten",
            vw_gate::dialog_quote(&path.display().to_string())
        ),
        (Some(path), true) => format!(
            "it is checked out in {}, so it cannot move",
            vw_gate::dialog_quote(&path.display().to_string())
        ),
        (None, _) => "only the branch pointer moves; no folder changes".to_owned(),
    }
}

fn pushable(state: &BranchState) -> Vec<RemoteInfo> {
    state
        .remotes
        .iter()
        .filter(|r| r.pushable)
        .cloned()
        .collect()
}

pub(crate) fn stale_unless(now: &str, seen: &str) -> Result<(), CoreError> {
    if now == seen {
        Ok(())
    } else {
        Err(git_error(vw_git::GitError::Stale(
            "The branch moved since the dialog opened. Open it again.".to_owned(),
        )))
    }
}

#[cfg(test)]
mod kg1_tests {
    use super::{PushPlan, bring_sentence, push_consent};

    #[test]
    fn bring_prompt_names_work_and_folder_without_commit_ids() {
        let plan = vw_git::merge::BringPlan {
            base: "main".into(),
            base_commit: "a".repeat(40),
            branch: "vibe/fix".into(),
            branch_commit: "b".repeat(40),
            kind: vw_git::merge::BringKind::FastForward,
            conflicts: vec![],
            checked_out: Some("/work/project".into()),
            checked_out_elsewhere: false,
            tree: None,
        };
        assert_eq!(
            bring_sentence(&plan, std::path::Path::new("/work/project")),
            "Bring the work from the copy \"vibe/fix\" into main in /work/project?\nIf you have unsaved changes there that would be overwritten, nothing is changed."
        );
    }

    #[test]
    fn push_summary_names_full_sanitized_target_and_commit() {
        let plan = PushPlan {
            remote: "origin".to_owned(),
            url: format!("https://example.test/{}", "long/".repeat(100)),
            branch: "feature\nApprove all".to_owned(),
            remote_branch: "main".to_owned(),
            commit: "a".repeat(40),
            commits: 2,
            first_push: false,
            lfs: false,
            target: "https://secret@example.test".to_owned(),
        };
        let text = push_consent(&plan);
        assert!(text.contains(&plan.url));
        assert!(text.contains(&plan.commit));
        assert!(!text.contains("secret@"));
        assert!(!text.contains("feature\nApprove"));
    }
}
