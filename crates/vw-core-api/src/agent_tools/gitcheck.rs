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
//! Invariant: nothing here runs git that writes; reads use the hardened runner.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use vw_agent_tools::gitparse::{CREDENTIAL_REASON, GH_WRITE_REASON, GitKind, GitScan, scan};
use vw_agent_tools::hook::PUSH_REASON;
use vw_changes::git::{Call, Index};
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

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
    let found = scan(command);
    if !found.needs_core() {
        return Ok(());
    }
    let worker = shared.clone();
    let id = session.to_owned();
    let folder = cwd.to_path_buf();
    let reading = tokio::task::spawn_blocking(move || own_branch(&worker, &id, &folder))
        .await
        .map_err(|e| format!("Not run: Vibe Work could not check this git command ({e})."))?;
    let own_branch = reading;
    let own_branch = own_branch.as_deref();
    let (verdict, how) = if found.forge_write() {
        (Err(GH_WRITE_REASON.to_owned()), "denied (forge write)")
    } else if found.denied() {
        (Err(PUSH_REASON.to_owned()), "denied (push)")
    } else if found.credential_read() {
        (Err(CREDENTIAL_REASON.to_owned()), "denied (credentials)")
    } else {
        let asks = needs_user(&found, own_branch);
        if asks.is_empty() {
            (Ok(()), "allowed")
        } else if matches!(asked, Asked::Card(true)) {
            // The user approved Vibe's card for this exact call of our own tool moments ago.
            (Ok(()), "allowed by the user on Vibe's card")
        } else if hold(shared, session, command, &asks).await {
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
        } else {
            (
                Err(format!(
                    "Not run: the user declined {} in Vibe Work's dialog. Continue without it, or ask the user.",
                    asks.join(", ")
                )),
                "refused by the user",
            )
        }
    };
    tracing::info!(session, ops = found.ops.len(), decision = how, "git guard");
    verdict
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

/// Holds the command for the user: a native dialog with the exact command.
async fn hold(shared: &Shared, session: &str, command: &str, asks: &[String]) -> bool {
    let title = shared
        .names
        .title_of(&shared.store, session)
        .unwrap_or_else(|| session.to_owned());
    let request = ConfirmationRequest {
        method: HOLD_METHOD.to_owned(),
        requester: vw_gate::Requester::Agent,
        summary: hold_summary(&title, command, asks),
        params: json!({"sessionId": session, "command": command, "why": asks}),
        always: None,
    };
    shared.confirmer.confirm(&request).await == Confirmation::Approved
}

fn hold_summary(title: &str, command: &str, asks: &[String]) -> String {
    format!(
        "The agent of the session {} wants to run a Git command that may rewrite a branch outside its own copy. It runs as you.\n\nWhy this was held: {}.\n\nCommand:\n{}",
        vw_gate::dialog_quote(title),
        asks.iter()
            .map(|reason| vw_gate::dialog_line(reason))
            .collect::<Vec<_>>()
            .join("; "),
        vw_gate::dialog_quote(command),
    )
}

#[cfg(test)]
mod kg1_tests {
    use super::hold_summary;

    #[test]
    fn held_git_command_is_complete() {
        let command = format!("git push --force {}", "a".repeat(2000));
        let text = hold_summary("Build session", &command, &["remote rewrite".to_owned()]);
        assert!(text.contains("Build session"));
        assert!(text.contains("remote rewrite"));
        assert!(text.ends_with(&format!("Command:\n{}", vw_gate::dialog_quote(&command))));
    }
}
