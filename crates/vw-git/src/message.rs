//! Commit messages and pull-request text: the repository's own style, the
//! prompt for the model that drafts a message, the draft used without a model, the optional credit line,
//! and the PR template read from `HEAD`.
//!
//! Depends on: `state` (read-only git), `error`. Called by: vw-core-api (`git` command: `draftMessage`,
//! `prPlan`).
//! Invariants: pure text work plus read-only git (`cat-file` of `HEAD`'s tree: a template is never read
//! from the working tree, which the agent may be changing).

use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::GitError;
use crate::state;

/// The credit line Vibe itself writes on the agent's commits (Vibe 2.26.1 `vibe/core/system_prompt.py:273`).
pub const CREDIT_LINE: &str = "Co-Authored-By: Mistral Vibe <vibe@mistral.ai>";

/// How the repository's recent subjects are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Style {
    /// Most subjects use Conventional Commits (`feat:`, `fix(scope):`).
    pub conventional: bool,
    /// Most subjects start with a capital letter.
    pub capitalized: bool,
    /// Most subjects end without a period.
    pub no_period: bool,
    /// The most common type (`feat`, `fix`,...) when conventional.
    pub common_type: Option<String>,
}

/// Reads the style of `subjects` (newest first). An empty list gives plain, capitalized, no period.
#[must_use]
pub fn style_of(subjects: &[String]) -> Style {
    let n = subjects.len();
    if n == 0 {
        return Style {
            conventional: false,
            capitalized: true,
            no_period: true,
            common_type: None,
        };
    }
    let mut types: Vec<(String, usize)> = Vec::new();
    let mut conventional = 0;
    for s in subjects {
        if let Some(kind) = conventional_type(s) {
            conventional += 1;
            match types.iter_mut().find(|(t, _)| *t == kind) {
                Some(entry) => entry.1 += 1,
                None => types.push((kind, 1)),
            }
        }
    }
    let capitalized = subjects
        .iter()
        .filter(|s| s.chars().next().is_some_and(char::is_uppercase))
        .count();
    let periods = subjects
        .iter()
        .filter(|s| s.trim_end().ends_with('.'))
        .count();
    types.sort_by_key(|t| std::cmp::Reverse(t.1));
    Style {
        conventional: conventional * 2 > n,
        capitalized: capitalized * 2 > n,
        no_period: periods * 2 <= n,
        common_type: types.first().map(|(t, _)| t.clone()),
    }
}

/// The Conventional Commits type of `subject` (`feat` of `feat(ui)!: x`), if it has one.
#[must_use]
pub fn conventional_type(subject: &str) -> Option<String> {
    let (head, _) = subject.split_once(": ")?;
    let head = head.trim_end_matches('!');
    let kind = head.split('(').next()?;
    (!kind.is_empty() && kind.len() <= 12 && kind.chars().all(|c| c.is_ascii_lowercase()))
        .then(|| kind.to_owned())
}

/// The message used without a model: the session title in the repository's style, then the file list.
#[must_use]
pub fn fallback(title: &str, style: &Style, files: &[String]) -> String {
    let mut subject = title.trim().trim_end_matches('.').to_owned();
    if subject.is_empty() {
        subject = match files {
            [one] => format!("Update {one}"),
            _ => format!("Update {} files", files.len()),
        };
    }
    if style.conventional && conventional_type(&subject).is_none() {
        let kind = style
            .common_type
            .clone()
            .unwrap_or_else(|| "chore".to_owned());
        let mut chars = subject.chars();
        let lowered = chars
            .next()
            .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
            .unwrap_or_default();
        subject = format!("{kind}: {lowered}");
    } else if style.capitalized {
        let mut chars = subject.chars();
        subject = chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default();
    }
    if !style.no_period {
        subject.push('.');
    }
    let subject: String = subject.chars().take(72).collect();
    let mut body = String::new();
    if files.len() > 1 {
        body.push_str("\n\n");
        for f in files.iter().take(20) {
            body.push_str("- ");
            body.push_str(f);
            body.push('\n');
        }
        if files.len() > 20 {
            let _ = writeln!(body, "- and {} more", files.len() - 20);
        }
    }
    format!("{subject}{}", body.trim_end())
}

/// Appends the credit line (once) when `credit` is on.
#[must_use]
pub fn with_credit(message: &str, credit: bool) -> String {
    let trimmed = message.trim_end();
    if !credit || trimmed.contains(CREDIT_LINE) {
        return trimmed.to_owned();
    }
    format!("{trimmed}\n\n{CREDIT_LINE}")
}

/// The system and user prompt for the model that drafts a commit message.
#[must_use]
pub fn prompt(
    subjects: &[String],
    instruction: &str,
    title: &str,
    stat: &str,
    diff: &str,
) -> (String, String) {
    let system = "You write git commit messages. Answer with the commit message only: a subject line of at most 72 \
                  characters, then, when the change needs it, one empty line and a short body of plain sentences or \
                  '- ' items. No code fences, no quotes around the message, no trailers. Follow the style of the \
                  repository's recent subjects exactly (prefixes such as 'feat:' or 'fix(scope):', capital letters, \
                  tense, final periods)."
        .to_owned();
    let mut user = String::new();
    if !instruction.trim().is_empty() {
        user.push_str("Project rule for commit messages (follow it):\n");
        user.push_str(instruction.trim());
        user.push_str("\n\n");
    }
    user.push_str("Recent subjects in this repository, newest first:\n");
    if subjects.is_empty() {
        user.push_str("(none yet)\n");
    }
    for s in subjects {
        user.push_str("- ");
        user.push_str(s);
        user.push('\n');
    }
    if !title.trim().is_empty() {
        user.push_str("\nThe task of the session that made the change: ");
        user.push_str(title.trim());
        user.push('\n');
    }
    user.push_str("\nFiles (git diff --stat):\n");
    user.push_str(stat);
    user.push_str("\n\nThe change (shortened):\n");
    user.push_str(diff);
    (system, user)
}

/// Cleans a model answer into a message: no code fences, no surrounding quotes, at most 72 characters in the
/// subject (longer ones are cut at a word). `None` when nothing usable is left.
#[must_use]
pub fn clean_answer(answer: &str) -> Option<String> {
    let mut text = answer.trim();
    if let Some(inner) = text.strip_prefix("```") {
        let inner = inner.split_once('\n').map_or("", |(_, rest)| rest);
        text = inner.trim_end().strip_suffix("```").unwrap_or(inner).trim();
    }
    let text = text
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim();
    let mut lines = text.lines();
    let subject = lines.next()?.trim();
    if subject.is_empty() {
        return None;
    }
    let subject = if subject.chars().count() > 72 {
        let cut: String = subject.chars().take(72).collect();
        cut.rsplit_once(' ')
            .map_or(cut.clone(), |(head, _)| head.to_owned())
    } else {
        subject.to_owned()
    };
    let body: Vec<&str> = lines.collect();
    let body = body.join("\n");
    let body = body.trim();
    Some(if body.is_empty() {
        subject
    } else {
        format!("{subject}\n\n{body}")
    })
}

/// Places a pull request template may live (GitHub, GitLab, Forgejo/Gitea), in the order they are tried.
pub const TEMPLATE_PATHS: &[&str] = &[
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "pull_request_template.md",
    "PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
    "docs/PULL_REQUEST_TEMPLATE.md",
    ".gitlab/merge_request_templates/Default.md",
    ".gitea/pull_request_template.md",
    ".forgejo/pull_request_template.md",
];

/// The repository's PR template at `HEAD` as `(path, text)`, if it has one (first match of
/// [`TEMPLATE_PATHS`], at most 64 KiB).
///
/// # Errors
/// git failures.
pub fn pr_template(root: &Path) -> Result<Option<(String, String)>, GitError> {
    for path in TEMPLATE_PATHS {
        let spec = format!("HEAD:{path}");
        if let Some(text) = state::read(root, &["cat-file", "-p", &spec])? {
            let text: String = text.chars().take(64 * 1024).collect();
            return Ok(Some(((*path).to_owned(), text)));
        }
    }
    Ok(None)
}

/// A PR title and body from the branch's commit subjects (oldest first) and the template. One commit: its
/// subject and body; several: the session title (or the first subject) and a list of the subjects. The
/// template follows the generated part, so its checklist stays for the author to fill.
#[must_use]
pub fn pr_text(
    title: &str,
    commits: &[(String, String)],
    template: Option<&str>,
) -> (String, String) {
    let (pr_title, mut body) = match commits {
        [(subject, body)] => (subject.clone(), body.trim().to_owned()),
        [] => (title.trim().to_owned(), String::new()),
        many => {
            let head = if title.trim().is_empty() {
                many[0].0.clone()
            } else {
                title.trim().to_owned()
            };
            let list = many
                .iter()
                .map(|(s, _)| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n");
            (head, list)
        }
    };
    if let Some(template) = template.map(str::trim).filter(|t| !t.is_empty()) {
        if body.is_empty() {
            template.clone_into(&mut body);
        } else {
            body = format!("{body}\n\n{template}");
        }
    }
    let pr_title: String = pr_title.chars().take(200).collect();
    (pr_title, body)
}

/// The branch's commits since `base` (oldest first) as `(subject, body)`.
///
/// # Errors
/// git failures.
pub fn branch_commits(root: &Path, base: &str) -> Result<Vec<(String, String)>, GitError> {
    let range = format!("{base}..HEAD");
    let text = state::read(
        root,
        &[
            "log",
            "--reverse",
            "--no-merges",
            "-n200",
            "--format=%s%x1f%b%x1e",
            &range,
        ],
    )?
    .unwrap_or_default();
    Ok(text
        .split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            let (subject, body) = record.split_once('\u{1f}')?;
            (!subject.trim().is_empty())
                .then(|| (subject.trim().to_owned(), body.trim().to_owned()))
        })
        .collect())
}
