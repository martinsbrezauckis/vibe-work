//! `git_check`: the git guard's question. The hook classified a bash command
//! (`vw_agent_tools::gitparse`); this decides on a fresh scan with what only the core knows:
//! the session's own copy branch.
//!
//! Vibe's access-mode card is the one ask for ordinary local git changes, including commits,
//! merges and pulls. Pushes, forge writes and credential commands are
//! refused. Branch rewrites outside the session's own copy branch are held for the user. **One ask per call (`one_ask`):**
//!   - a bash call (the hook runs before Vibe's card): a yes here is remembered for the call, and the core answers
//!     Vibe's card for it with that yes (the card is not shown);
//!   - our own `process_start`/`secret_run` (Vibe's card comes first): when the user approved that card for this
//!     exact call, the guard does not ask again (allow);
//! - everything else -> **allow** (Vibe's card, if the mode asks).
//!
//! A guard against mistakes, not a security boundary (see `gitparse`). Depends on: vw-agent-tools (`gitparse`,
//! `hook::PUSH_REASON`), vw-changes (the hardened runner for branch reads), vw-gate (confirmation),
//! `worktrees` (records), `one_ask`. Called by: `server` (hook credential only), `tools` (`process_start`),
//! `secret_run`.
//! Invariant: nothing here runs git that writes; reads use the hardened runner. An allowed own rename
//! records its intent; the worktree record follows once the rename is verified in that exact copy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use vw_agent_tools::gitparse::{CREDENTIAL_REASON, GH_WRITE_REASON, GitKind, GitScan, scan};
use vw_agent_tools::hook::PUSH_REASON;
use vw_changes::git::{Call, Index};
use vw_gate::ConfirmationRequest;

use crate::core::Shared;
use crate::one_ask::Asked;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Question {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    parent_session_id: Option<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    command: String,
    /// The hook's `tool_call_id` (empty from a hook that predates it: then Vibe's card is shown too).
    #[serde(default)]
    tool_call_id: String,
}

/// The method name a held command is shown under in the native dialog.
pub(crate) const HOLD_METHOD: &str = "vibe-work/gitGuard";

/// Answers one `git_check`: `{"decision": "allow"}` or `{"decision": "deny", "reason": ...}`.
pub(crate) async fn check(shared: &Arc<Shared>, args: Value) -> Result<Value, String> {
    let question: Question =
        serde_json::from_value(args).map_err(|_| "bad git_check request".to_owned())?;
    let session = if shared.store.view(&question.session_id).is_some() {
        question.session_id.clone()
    } else {
        question
            .parent_session_id
            .clone()
            .filter(|p| shared.store.view(p).is_some())
            .unwrap_or_else(|| question.session_id.clone())
    };
    let cwd = PathBuf::from(&question.cwd);
    Ok(
        match guard(
            shared,
            &session,
            &cwd,
            &question.command,
            Asked::Hook {
                call_id: &question.tool_call_id,
                hook_session: &question.session_id,
            },
        )
        .await
        {
            Ok(()) => json!({"decision": "allow"}),
            Err(reason) => json!({"decision": "deny", "reason": reason}),
        },
    )
}

/// The one git-guard decision, shared by the bash hook, `process_start` and `secret_run`. `Ok(())` = run it.
/// `asked`: who else asks the user about this call (module docs).
///
/// # Errors
/// The refusal text for the agent.
pub(crate) async fn guard(
    shared: &Arc<Shared>,
    session: &str,
    cwd: &Path,
    command: &str,
    asked: Asked<'_>,
) -> Result<(), String> {
    decide(shared, session, cwd, command, asked)
        .await
        .map_err(|reason| {
            if matches!(asked, Asked::Hook { .. }) || reason.starts_with("Not done:") {
                reason
            } else {
                crate::guard_refusals::record(
                    shared,
                    session,
                    crate::GuardKind::Git,
                    crate::guard_refusals::step(shared, session, None),
                    &reason,
                )
            }
        })
}

async fn decide(
    shared: &Arc<Shared>,
    session: &str,
    cwd: &Path,
    command: &str,
    asked: Asked<'_>,
) -> Result<(), String> {
    let mut found = scan(command);
    if shared.github_reads.enabled() {
        found.ops.retain(|op| !op.what.starts_with("gh "));
    }
    if !found.needs_core() {
        return Ok(());
    }
    let worker = shared.clone();
    let id = session.to_owned();
    let folder = cwd.to_path_buf();
    let mut checked = found.clone();
    // Only a rename that runs in this folder can be checked here: after a `cd` (any reading saying so) the target
    // lives in another repository, and the rename is held as another branch's rewrite anyway.
    let mut renames: Vec<String> = found
        .ops
        .iter()
        .filter_map(|op| op.rename_to.clone())
        .filter(|to| {
            !found
                .ops
                .iter()
                .any(|op| op.elsewhere && op.rename_to.as_ref() == Some(to))
        })
        .collect();
    renames.sort();
    renames.dedup();
    let reading = tokio::task::spawn_blocking(move || {
        let own = own_branch(&worker, &id, &folder);
        for op in &mut checked.ops {
            if !own_paths(&folder, op.git_dir.as_deref(), op.work_tree.as_deref()) {
                op.elsewhere = true;
            }
        }
        let occupied: Vec<String> = renames
            .into_iter()
            .filter(|to| own.as_deref() != Some(to.as_str()))
            .filter_map(|to| {
                let code = Call::new(&folder, Index::User)
                    .args([
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{to}"),
                    ])
                    .timeout(Duration::from_secs(5))
                    .output()
                    .ok()
                    .and_then(|out| out.code);
                target_reason(code, &to)
            })
            .collect();
        (checked, own, occupied)
    })
    .await
    .map_err(|e| format!("Not run: Vibe Work could not check this git command ({e})."))?;
    let (found, own_branch, occupied) = reading;
    let own_branch = own_branch.as_deref();
    let (verdict, how) = if found.forge_write() {
        (Err(GH_WRITE_REASON.to_owned()), "denied (forge write)")
    } else if found.denied() {
        (Err(PUSH_REASON.to_owned()), "denied (push)")
    } else if found.credential_read() {
        (Err(CREDENTIAL_REASON.to_owned()), "denied (credentials)")
    } else {
        let mut asks = needs_user(&found, own_branch);
        asks.extend(occupied);
        if asks.is_empty() {
            remember_rename(shared, session, &found, own_branch).await?;
            (Ok(()), "allowed")
        } else if matches!(asked, Asked::Card(true)) {
            remember_rename(shared, session, &found, own_branch).await?;
            // The user approved Vibe's card for this exact call of our own tool moments ago.
            (Ok(()), "allowed by the user on Vibe's card")
        } else {
            match hold(shared, session, cwd, command, &asks, asked).await {
                Ok(()) => {
                    remember_rename(shared, session, &found, own_branch).await?;
                    if let Asked::Hook {
                        call_id,
                        hook_session,
                    } = asked
                    {
                        // Vibe's card for this bash call (if its permissions ask) gets this yes from the core.
                        if let Some(ledger) = crate::one_ask::of(shared) {
                            ledger.guard_said_yes(&[hook_session, session], call_id, command);
                        }
                    }
                    (Ok(()), "allowed by the user")
                }
                Err(reason) => (Err(reason), "refused"),
            }
        }
    };
    tracing::info!(session, ops = found.ops.len(), decision = how, "git guard");
    verdict
}

/// Why a rename onto `to` is held, from `git show-ref --verify --quiet`'s exit code: 0 = that branch exists, 1 = it
/// does not (`None`, nothing to say); anything else (no repository here, git failed) = unknown, still held, and
/// said as such instead of claiming the branch exists.
fn target_reason(code: Option<i32>, to: &str) -> Option<String> {
    match code {
        Some(0) => Some(format!("renaming onto existing branch `{to}`")),
        Some(1) => None,
        _ => Some(format!(
            "renaming to `{to}`, which Vibe Work could not check for an existing branch"
        )),
    }
}

fn git_path(cwd: &Path, args: &[&str]) -> Option<PathBuf> {
    let out = Call::new(cwd, Index::User)
        .args(args)
        .timeout(Duration::from_secs(5))
        .output()
        .ok()?;
    if !out.ok() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    path.canonicalize().ok()
}

fn own_paths(cwd: &Path, git_dir: Option<&str>, work_tree: Option<&str>) -> bool {
    let directory_matches = git_dir.is_none_or(|path| {
        let selected = git_path(cwd, &["rev-parse", "--resolve-git-dir", path]);
        selected.is_some() && selected == git_path(cwd, &["rev-parse", "--absolute-git-dir"])
    });
    let tree_matches = work_tree.is_none_or(|path| {
        let selected = cwd.join(path).canonicalize().ok();
        selected.is_some() && selected == git_path(cwd, &["rev-parse", "--show-toplevel"])
    });
    directory_matches && tree_matches
}

async fn remember_rename(
    shared: &Arc<Shared>,
    session: &str,
    found: &GitScan,
    own: Option<&str>,
) -> Result<(), String> {
    let Some(to) = found.ops.iter().find_map(|op| {
        (own.is_some() && !op.elsewhere && op.name.as_deref().is_none_or(|name| Some(name) == own))
            .then(|| op.rename_to.clone())
            .flatten()
    }) else {
        return Ok(());
    };
    let shared = shared.clone();
    let session = session.to_owned();
    tokio::task::spawn_blocking(move || {
        let records = shared.worktrees.records().map_err(|e| e.to_string())?;
        let record = records.get(&session).ok_or("the copy record is missing")?;
        let copy = crate::worktrees::find_path(&record.repo_root, &record.branch)
            .ok_or("the copy is missing")?;
        records
            .note_rename(&session, &to, &copy)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Not run: the copy's branch rename could not be recorded ({e})."))
}

/// The branch Vibe Work created for this session's copy (record) when `cwd`'s checkout is on it, else `None`.
fn own_branch(shared: &Shared, session: &str, cwd: &Path) -> Option<String> {
    let records = shared.worktrees.records().ok()?;
    let record = records.get(session)?;
    if record.decision == Some(vw_worktrees::Decision::Removed) {
        return None;
    }
    // The branch name alone is insufficient: another checkout could carry the same name later.
    let copy = crate::worktrees::find_path(&record.repo_root, &record.branch)?
        .canonicalize()
        .ok()?;
    let here = cwd.canonicalize().ok()?;
    if !here.starts_with(&copy) {
        return None;
    }
    let top = Call::new(cwd, Index::User)
        .args(["rev-parse", "--show-toplevel"])
        .timeout(Duration::from_secs(5))
        .output()
        .ok()?;
    if !top.ok()
        || Path::new(String::from_utf8_lossy(&top.stdout).trim())
            .canonicalize()
            .ok()
            .as_deref()
            != Some(copy.as_path())
    {
        return None;
    }
    let out = Call::new(cwd, Index::User)
        .args(["symbolic-ref", "-q", "--short", "HEAD"])
        .timeout(Duration::from_secs(5))
        .output()
        .ok()?;
    (out.ok() && String::from_utf8_lossy(&out.stdout).trim() == record.branch)
        .then(|| record.branch.clone())
}

/// The actions the user has to allow (empty = none): another branch's rewrite.
/// Ordinary local changes use Vibe's card; refused actions never get here.
fn needs_user(found: &GitScan, own_branch: Option<&str>) -> Vec<String> {
    let mut asks: Vec<String> = found
        .ops
        .iter()
        .filter(|o| match o.kind {
            GitKind::Deny | GitKind::Credential | GitKind::Change => false,
            // An explicit target (`git rebase main other`, `git branch -f x`) is held
            // unless it names the session's branch.
            GitKind::Own => {
                own_branch.is_none()
                    || o.elsewhere
                    || o.name.as_deref().is_some_and(|b| Some(b) != own_branch)
            }
        })
        .map(|o| format!("`{}`", o.what))
        .collect();
    asks.dedup();
    asks
}

fn command_chat<'a>(session: &'a str, asked: Asked<'a>) -> &'a str {
    match asked {
        Asked::Hook { hook_session, .. } => hook_session,
        Asked::Card(_) => session,
    }
}

/// Holds the command for the user: a native dialog with the exact command.
async fn hold(
    shared: &Arc<Shared>,
    session: &str,
    cwd: &Path,
    command: &str,
    asks: &[String],
    asked: Asked<'_>,
) -> Result<(), String> {
    let title = shared.names.confirmation_title(&shared.store, session);
    // A child may borrow its parent's worktree record, but never its exact-command approval identity.
    let session = command_chat(session, asked);
    let mut request = ConfirmationRequest {
        method: HOLD_METHOD.to_owned(),
        requester: vw_gate::Requester::Agent,
        summary: hold_summary(&title, command, asks),
        params: json!({"sessionId": session, "cwd": cwd, "command": command, "why": asks}),
        always: None,
    };
    if let Asked::Hook { call_id, .. } = asked {
        request.params["toolCallId"] = Value::String(call_id.into());
    }
    crate::ask::guards::wait(shared, request, "this Git command").await
}

fn hold_summary(title: &str, command: &str, asks: &[String]) -> String {
    format!(
        "The agent of the session {} wants to run a Git command that may rewrite a branch outside its own copy. It runs as you.\n\nWhy this was held: {}.\n\nCommand:\n{}",
        vw_gate::dialog_chat_quote(title),
        asks.iter()
            .map(|reason| vw_gate::dialog_line(reason))
            .collect::<Vec<_>>()
            .join("; "),
        vw_gate::dialog_command(command),
    )
}

#[cfg(test)]
mod kg1_tests {
    use super::{hold_summary, target_reason};

    #[test]
    fn a_rename_target_is_called_existing_only_when_git_says_so() {
        assert_eq!(
            target_reason(Some(0), "spare-2").as_deref(),
            Some("renaming onto existing branch `spare-2`")
        );
        assert_eq!(target_reason(Some(1), "spare-2"), None);
        for unknown in [Some(128), None] {
            let reason = target_reason(unknown, "spare-2").unwrap();
            assert!(!reason.contains("existing branch `"), "{reason}");
            assert!(reason.contains("could not check"), "{reason}");
        }
    }

    #[test]
    fn held_git_command_is_complete() {
        let command = format!("git push --force {}", "a".repeat(2000));
        let text = hold_summary("Build session", &command, &["remote rewrite".to_owned()]);
        assert!(text.contains("Build session"));
        assert!(text.contains("remote rewrite"));
        assert!(text.ends_with(&format!("Command:\n{command}")));
    }
}
