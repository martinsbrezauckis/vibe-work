//! The pull-request half of the `git` command: the PR
//! dialog's plan (forge, gh or the browser, base, title and body from the commits and the template, the push that
//! has to happen first) and creating it from one immutable [`PrAction`]: the native dialog names exactly that
//! action, and right before it runs the branch, its commit and the remote are checked against it.
//! The PR card and the prepared messages are in `git_card.rs`; Push and "Bring into main" in `git_publish.rs`.
//!
//! Depends on: vw-git, `git_cmd` (place, errors), `git_publish` (`requester`, push), `git_card`. Called by:
//! `git_publish`.
//! Invariant: gh is asked only for a user's action (the dialog, Create PR).

use serde_json::json;
use vw_git::forge::{self, ForgeKind};
use vw_git::gh::Gh;
use vw_git::message;
use vw_git::push;
use vw_git::state::{self, BranchState, RemoteInfo};

use crate::command::Origin;
use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::{Place, git_error, state_blocker};
use crate::git_publish::{folder_label, push_consent, requester, stale_unless};
use crate::git_view::{GitRequest, GitResult, PrCreatedView, PrPlanView, PrVia};
use crate::worktrees::blocking;

impl Core {
    pub(crate) fn gh(&self) -> Option<Gh> {
        let workdir = self.shared.git.folder("gh").ok()?;
        vw_git::gh::find(self.shared.config.gh_program.as_deref(), &workdir)
    }

    pub(crate) async fn pr_plan(
        &self,
        session_id: &str,
        place: &Place,
        remote: Option<String>,
    ) -> Result<PrPlanView, CoreError> {
        let p = place.clone();
        let wanted = remote.clone();
        let (state, remote_info) = blocking(move || {
            let state = state::read_state(&p.root, &p.git_dir, p.base.as_deref())?;
            let info = match wanted.as_deref() {
                Some(name) => state.remotes.iter().find(|r| r.name == name).cloned(),
                None => state
                    .default_remote()
                    .filter(|r| supports_pr(r))
                    .or_else(|| state.remotes.iter().find(|r| supports_pr(r)))
                    .or_else(|| state.default_remote())
                    .cloned(),
            };
            Ok::<_, vw_git::GitError>((state, info))
        })
        .await?
        .map_err(git_error)?;
        if let Some(why) = state_blocker(&state) {
            return Err(git_error(vw_git::GitError::Blocked(why)));
        }
        let remote_info = remote_info.ok_or_else(|| {
            git_error(vw_git::GitError::Blocked(
                "This repository has no remote to open a pull request on. Add one in your git tool.".to_owned(),
            ))
        })?;
        let head = state.branch().unwrap_or("").to_owned();
        let base = self.pr_base(place, &state, &remote_info).await;
        let p = place.clone();
        let (base2, remote_name, st) = (base.clone(), remote_info.name.clone(), state.clone());
        let (commits, template, push_plan) = blocking(move || {
            // The branch's own commits: since the remote's base if it was fetched, else the local base.
            let tracking = format!("refs/remotes/{remote_name}/{base2}");
            let known = vw_changes::git::Call::new(&p.root, vw_changes::git::Index::User)
                .args(["rev-parse", "--verify", "-q", &tracking])
                .output()
                .is_ok_and(|o| o.ok());
            let from = if known {
                tracking
            } else {
                format!("refs/heads/{base2}")
            };
            let commits = message::branch_commits(&p.root, &from).unwrap_or_default();
            let template = message::pr_template(&p.root).ok().flatten();
            (
                commits,
                template,
                push::plan(&p.root, &st, Some(&remote_name)),
            )
        })
        .await?;
        let push_plan = push_plan.map_err(git_error)?;
        let needs = push_plan.first_push || push_plan.commits > 0;
        let (title, body) = message::pr_text(
            &place.title,
            &commits,
            template.as_ref().map(|(_, t)| t.as_str()),
        );
        let forge = remote_info.forge.clone();
        let (via, gh_note) = self.pr_via(forge.as_ref()).await;
        let existing = self
            .shared
            .git
            .card(session_id, &place.root)
            .card
            .filter(|c| c.head == head);
        let push_part = if needs {
            format!("{} Then ", push_consent(&push_plan).trim_end_matches('?'))
        } else {
            String::new()
        };
        let target = forge
            .as_ref()
            .map_or_else(|| remote_info.push_url.clone(), forge::Forge::web);
        let sentence = match via {
            PrVia::Gh => format!(
                "{push_part}create a pull request from {head} into {base} on {target} with gh?"
            ),
            PrVia::Browser => {
                format!("{push_part}open the new pull request page of {target} in your browser?")
            }
            PrVia::None => "Local remote".to_owned(),
        };
        let sentence = format!("In folder {}: {sentence}", folder_label(place));
        Ok(PrPlanView {
            remote: remote_info.name.clone(),
            forge,
            via,
            gh_note,
            base,
            head,
            title,
            body,
            template: template.map(|(p, _)| p),
            push: needs.then_some(push_plan),
            existing,
            sentence,
        })
    }

    /// The PR's base: the copy's base, else the remote's default branch, else `main`.
    async fn pr_base(&self, place: &Place, _state: &BranchState, remote: &RemoteInfo) -> String {
        if let Some(base) = &place.base {
            return base.clone();
        }
        let (root, name) = (place.root.clone(), remote.name.clone());
        blocking(move || state::remote_default_branch(&root, &name).ok().flatten())
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "main".to_owned())
    }

    async fn pr_via(&self, forge: Option<&forge::Forge>) -> (PrVia, Option<String>) {
        let Some(forge) = forge else {
            return (PrVia::None, None);
        };
        match forge.kind {
            ForgeKind::GitHub => match self.gh() {
                Some(gh) => {
                    let host = forge.host.clone();
                    let signed = blocking(move || gh.signed_in(&host))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false);
                    if signed {
                        (PrVia::Gh, None)
                    } else {
                        (PrVia::Browser, Some("gh is installed but not signed in (gh auth login): GitHub's page opens instead.".to_owned()))
                    }
                }
                None => (
                    PrVia::Browser,
                    Some("gh is not installed: GitHub's page opens in your browser.".to_owned()),
                ),
            },
            ForgeKind::GitLab | ForgeKind::Forgejo => (PrVia::Browser, None),
            ForgeKind::Unknown => (PrVia::None, None),
        }
    }

    /// Creates the pull request from one immutable [`PrAction`]: the native dialog shows exactly it, and right
    /// before anything runs the branch, its commit and the remote's address are checked against it:
    /// what the dialog says is what happens; anything changed is refused, nothing is created).
    pub(crate) async fn create_pr(
        &self,
        origin: Origin,
        session_id: &str,
        place: &Place,
        action: PrAction,
    ) -> Result<PrCreatedView, CoreError> {
        ensure_pr_route(action.via)?;
        let sentence = format!("In folder {}: {}", folder_label(place), action.sentence());
        if action.via == PrVia::Gh || action.push.is_some() {
            let params = serde_json::to_value(&action).unwrap_or_default();
            self.ask_native(
                requester(origin),
                "vibe-work/gitPullRequest",
                json!({"sessionId": session_id, "root": place.root, "action": params}),
                sentence,
            )
            .await?;
        }
        self.git_validate_place(session_id, place)?;
        self.still_as_confirmed(place, &action).await?;
        let mut pushed = String::new();
        if let Some(push_plan) = action.push.clone() {
            pushed = self.run_push(place, push_plan).await?;
            pushed.push(' ');
        }
        match (action.via, action.forge.clone()) {
            (PrVia::Gh, Some(forge)) => {
                let gh = self.gh().ok_or_else(|| {
                    CoreError::new(ErrorCode::Internal, "gh is no longer installed")
                })?;
                let a = action.clone();
                let url = blocking(move || {
                    gh.create(&forge, &a.head, &a.base, &a.title, &a.body, a.draft)
                })
                .await?
                .map_err(git_error)?;
                // The card follows at once (gh is fresh).
                let _ = self.pr_status(session_id, place, true).await;
                Ok(PrCreatedView {
                    via: PrVia::Gh,
                    url: Some(url.clone()),
                    message: format!("{pushed}Pull request created: {url}"),
                })
            }
            (_, Some(forge)) => {
                let url = forge::new_request_url(
                    &forge,
                    &action.base,
                    &action.head,
                    &action.title,
                    &action.body,
                );
                let note = if url.as_deref().is_some_and(forge::url_carries_body)
                    || forge.kind != ForgeKind::GitHub
                {
                    ""
                } else {
                    " The description was too long for the address: GitHub fills in the template; paste yours there."
                };
                let draft_note = if action.draft && forge.kind == ForgeKind::GitHub {
                    " Choose \"Create draft pull request\" on the page."
                } else {
                    ""
                };
                Ok(PrCreatedView {
                    via: if url.is_some() {
                        PrVia::Browser
                    } else {
                        PrVia::None
                    },
                    url,
                    message: format!(
                        "{pushed}Finish the pull request on the page that opens.{note}{draft_note}"
                    ),
                })
            }
            (_, None) => Ok(PrCreatedView {
                via: PrVia::None,
                url: None,
                message: format!("{pushed}No supported forge page is available for this remote."),
            }),
        }
    }

    /// Refuses when the branch, its commit or the remote's address is no longer what `action` names.
    async fn still_as_confirmed(&self, place: &Place, action: &PrAction) -> Result<(), CoreError> {
        let p = place.clone();
        let state = blocking(move || state::read_state(&p.root, &p.git_dir, None))
            .await?
            .map_err(git_error)?;
        let remote = state.remotes.iter().find(|r| r.name == action.remote);
        let same = state.branch() == Some(action.head.as_str())
            && state.commit.as_deref() == Some(action.commit.as_str())
            && remote.is_some_and(|r| r.push_url == action.push_url && r.forge == action.forge);
        if same {
            Ok(())
        } else {
            Err(git_error(vw_git::GitError::Stale(
                "The branch, its commit or the remote changed since you confirmed — review again."
                    .to_owned(),
            )))
        }
    }
}

/// One pull request exactly as the dialog confirms it and as it then runs (built once, never changed).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrAction {
    /// gh, the browser, or neither.
    pub(crate) via: PrVia,
    /// The forge repository.
    pub(crate) forge: Option<forge::Forge>,
    /// The remote's name.
    pub(crate) remote: String,
    /// Where a push goes (no user info).
    pub(crate) push_url: String,
    /// The head branch.
    pub(crate) head: String,
    /// Its commit when confirmed.
    pub(crate) commit: String,
    /// The base branch.
    pub(crate) base: String,
    /// The title.
    pub(crate) title: String,
    /// The body.
    pub(crate) body: String,
    /// A draft.
    pub(crate) draft: bool,
    /// The push that runs first, if any.
    pub(crate) push: Option<vw_git::push::PushPlan>,
}

impl PrAction {
    /// The confirmation's sentence: every value that then runs.
    pub(crate) fn sentence(&self) -> String {
        let push_part = self
            .push
            .as_ref()
            .map(|p| format!("{} Then ", push_consent(p).trim_end_matches('?')))
            .unwrap_or_default();
        let target = self
            .forge
            .as_ref()
            .map_or_else(|| self.push_url.clone(), forge::Forge::web);
        let target = vw_gate::dialog_quote(&target);
        let head = vw_gate::dialog_quote(&self.head);
        let base = vw_gate::dialog_quote(&self.base);
        let kind = if self.draft {
            "a draft pull request"
        } else {
            "a pull request"
        };
        let title = vw_gate::dialog_quote(&self.title);
        let body = vw_gate::dialog_quote(&self.body);
        let commit = vw_gate::dialog_quote(&self.commit);
        match self.via {
            PrVia::Gh => format!(
                "{push_part}create {kind} from {head} into {base} on {target} with gh?\n\nTitle: {title}\nBody: {body}\nCommit: {commit}"
            ),
            PrVia::Browser => format!(
                "{push_part}open the new pull request page ({head} into {base}) of {target} in your browser?\n\nTitle: {title}\nBody: {body}\nCommit: {commit}"
            ),
            PrVia::None => "Local remote".to_owned(),
        }
    }
}

impl Core {
    /// `createPr`: builds the one [`PrAction`] from the request and the folder now, then [`Core::create_pr`].
    pub(crate) async fn create_pr_request(
        &self,
        origin: Origin,
        request: GitRequest,
    ) -> Result<GitResult, CoreError> {
        let GitRequest::CreatePr {
            session_id,
            repo,
            remote,
            base,
            title,
            body,
            draft,
            commit,
        } = request
        else {
            return Err(CoreError::new(
                ErrorCode::Internal,
                "not a createPr request",
            ));
        };

        let (_, place) = self.git_selected_place(&session_id, repo.as_deref())?;
        let plan = self.pr_plan(&session_id, &place, Some(remote)).await?;
        if !vw_git::names::valid_ref_name(&base) {
            return Err(git_error(vw_git::GitError::Blocked(
                "The base branch name cannot be used.".to_owned(),
            )));
        }
        let p = place.clone();
        let state = blocking(move || vw_git::state::read_state(&p.root, &p.git_dir, None))
            .await?
            .map_err(git_error)?;
        let now = state.commit.clone().unwrap_or_default();
        stale_unless(&now, &commit)?;
        let push_url = state
            .remotes
            .iter()
            .find(|r| r.name == plan.remote)
            .map(|r| r.push_url.clone())
            .unwrap_or_default();
        let action = crate::git_pr::PrAction {
            via: plan.via,
            forge: plan.forge.clone(),
            remote: plan.remote.clone(),
            push_url,
            head: plan.head.clone(),
            commit: now,
            base,
            title,
            body,
            draft,
            push: plan.push.clone(),
        };
        let created = self.create_pr(origin, &session_id, &place, action).await?;
        Ok(GitResult::PrCreated { created })
    }
}

fn supports_pr(remote: &RemoteInfo) -> bool {
    remote.pushable
        && remote
            .forge
            .as_ref()
            .is_some_and(|f| f.kind != ForgeKind::Unknown)
}

fn ensure_pr_route(via: PrVia) -> Result<(), CoreError> {
    if via == PrVia::None {
        Err(git_error(vw_git::GitError::Blocked(
            "Local remote: Create PR needs a supported forge remote.".to_owned(),
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_pr_route, supports_pr};
    use vw_git::forge::{Forge, ForgeKind};
    use vw_git::state::RemoteInfo;

    #[test]
    fn local_and_unknown_remotes_cannot_route_a_pr() {
        let mut remote = RemoteInfo {
            name: "origin".to_owned(),
            push_url: "/tmp/local.git".to_owned(),
            forge: None,
            pushable: true,
        };
        assert!(!supports_pr(&remote));
        remote.forge = Some(Forge {
            kind: ForgeKind::Unknown,
            host: "example.invalid".to_owned(),
            path: "o/r".to_owned(),
        });
        assert!(!supports_pr(&remote));
        remote.forge.as_mut().unwrap().kind = ForgeKind::GitHub;
        assert!(supports_pr(&remote));
        remote.pushable = false;
        assert!(!supports_pr(&remote));

        let blocked = ensure_pr_route(super::PrVia::None).unwrap_err();
        assert!(blocked.message.contains("Local remote"));
        assert!(ensure_pr_route(super::PrVia::Browser).is_ok());
        assert!(ensure_pr_route(super::PrVia::Gh).is_ok());
    }
}
