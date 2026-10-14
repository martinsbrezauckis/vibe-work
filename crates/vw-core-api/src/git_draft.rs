//! The commit message draft of the `git` command: Mistral writes it from the
//! repository's recent subjects (style), the per-project instruction, the session title and a short diff, with
//! the user's key (the core's own direct call, like OCR; no unlock prompt is raised); without a key or on any
//! failure the draft is the session title in the repository's style, and the answer says which.
//!
//! Depends on: vw-git (`message`, `commit`, `programs`), vw-mistral (`chat`), `git_cmd`. Called by: `git_cmd`.
//! Invariant: git reads the working tree here only after the repository-program check.

use std::path::Path;
use std::time::Duration;

use vw_git::commit;
use vw_git::message;

use crate::core::Core;
use crate::error::CoreError;
use crate::git_view::{DraftSource, GitResult};
use crate::worktrees::blocking;

impl Core {
    pub(crate) async fn git_draft(
        &self,
        session_id: &str,
        repo: Option<&str>,
        paths: Vec<String>,
    ) -> Result<GitResult, CoreError> {
        let (_, place) = self.git_selected_place(session_id, repo)?;
        let settings = self.git_settings(&place.main_root);
        let p = place.clone();
        let wanted = paths.clone();
        let (subjects, stat, diff) = blocking(move || {
            let subjects = commit::recent_subjects(&p.root, 15).unwrap_or_default();
            let (stat, diff) = draft_diff(&p.root, &wanted);
            (subjects, stat, diff)
        })
        .await?;
        let style = message::style_of(&subjects);
        let fallback = message::with_credit(
            &message::fallback(&place.title, &style, &paths),
            settings.credit,
        );
        let Some(keys) = self.shared.config.keys.clone() else {
            return Ok(draft(
                fallback,
                DraftSource::Fallback,
                Some("No Mistral key in this run."),
            ));
        };
        let (system, user) =
            message::prompt(&subjects, &settings.instruction, &place.title, &stat, &diff);
        let request = vw_mistral::chat::ChatRequest {
            model: &self.shared.config.commit_model,
            system: &system,
            user: &user,
            max_tokens: 400,
            timeout: Duration::from_secs(40),
            // A commit message needs no thinking; Large 4 thinks by default and would spend the cap on it.
            reasoning_effort: Some("none"),
        };
        match keys.chat(request).await {
            Ok(Some(answer)) => match message::clean_answer(&answer) {
                Some(text) => Ok(draft(
                    message::with_credit(&text, settings.credit),
                    DraftSource::Model,
                    None,
                )),
                None => Ok(draft(
                    fallback,
                    DraftSource::Fallback,
                    Some("The model's answer was empty."),
                )),
            },
            Ok(None) => Ok(draft(
                fallback,
                DraftSource::Fallback,
                Some("Unlock or set up your Mistral key to get a written message."),
            )),
            Err(error) => Ok(draft(
                fallback,
                DraftSource::Fallback,
                Some(&format!(
                    "The message model could not be reached ({error})."
                )),
            )),
        }
    }
}

fn draft(message: String, source: DraftSource, note: Option<&str>) -> GitResult {
    GitResult::Draft {
        message,
        source,
        note: note.map(str::to_owned),
    }
}

/// A short diff for the message model: `--stat` and at most 12 KB of the patch of `paths` against `HEAD` (no
/// textconv, no external diff), plus the file list (new files have no diff against `HEAD`). Only after the
/// repository-program check; empty diff text when it fails.
fn draft_diff(root: &Path, paths: &[String]) -> (String, String) {
    use vw_changes::git::{Call, Index, Profile};
    let list = paths
        .iter()
        .map(|p| format!(" {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    if vw_git::programs::check(root, vw_git::programs::Purpose::Worktree).is_err() {
        return (list, String::new());
    }
    let run = |args: &[&str]| -> String {
        Call::new(root, Index::User)
            .profile(Profile::Worktree)
            .args([
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
            ])
            .args(args)
            .args(paths)
            .timeout(Duration::from_secs(20))
            .max_output(256 * 1024)
            .run()
            .map(|o| String::from_utf8_lossy(&o).into_owned())
            .unwrap_or_default()
    };
    let stat = run(&["--stat=100", "HEAD", "--"]);
    let diff: String = run(&["-U2", "HEAD", "--"]).chars().take(12_000).collect();
    (format!("{stat}\nAll files in this commit:\n{list}"), diff)
}
