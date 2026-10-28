//! Classifies literal Git and GitHub CLI command starts for the hook and core.
//! Depends on `command_segments`, `killparse_shell` (quote-aware readings) and `gitparse_cmds`; called by the hook,
//! core and tests.
//! This is a guardrail: it does not interpret scripts or words in command data. Each command is read as unquoted literal words and as Bash and Windows
//! dequoted words and every operation any reading finds counts, so a
//! quoted value with a space (`git -C "C:\work\git test" push`) or a quoted program name cannot hide a push
//! (a literal reading could otherwise take `"C:\...\git` as the `-C` value).

use crate::command_segments::{command_words, executable_name, segments};
use serde::{Deserialize, Serialize};

/// Policy for a recognized literal operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitKind {
    /// Push or GitHub CLI write.
    Deny,
    /// Branch rewrite, subject to own-branch check.
    Own,
    /// Ordinary local operation, handled by Vibe's access-mode card.
    Change,
    /// Credential command.
    Credential,
}

/// Reason for refusing credential commands.
pub const CREDENTIAL_REASON: &str = "Not run: the agent cannot use the user's stored Git or GitHub sign-in. Continue without reading credentials; the user can push or create a PR in the Changes tab.";
/// Reason for refusing a GitHub CLI command while reads are off.
pub const GH_WRITE_REASON: &str = "GitHub reads are off. The user can turn them on in Settings > General. For GitHub data use the GitHub connector.";

/// One recognized Git or GitHub CLI operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOp {
    /// Operation policy.
    pub kind: GitKind,
    /// Short human-readable command name.
    pub what: String,
    /// Global Git option selected another repository.
    pub elsewhere: bool,
    /// Explicit branch target for a rewrite.
    pub name: Option<String>,
    /// Destination of a branch rename; the source is `name` or the current branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename_to: Option<String>,
    /// Explicit Git directory, resolved by the core before an own-branch rewrite is released.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_dir: Option<String>,
    /// Explicit worktree root, resolved by the core before an own-branch rewrite is released.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_tree: Option<String>,
}

/// Recognized literal command starts in one shell input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitScan {
    /// Whether a recognized command starts with Git or GitHub CLI.
    pub mentions: bool,
    /// Recognized operations.
    pub ops: Vec<GitOp>,
}

impl GitScan {
    #[must_use]
    /// True for a GitHub CLI write.
    pub fn forge_write(&self) -> bool {
        self.ops
            .iter()
            .any(|o| o.kind == GitKind::Deny && o.what.starts_with("gh "))
            && !self
                .ops
                .iter()
                .any(|o| o.kind == GitKind::Deny && !o.what.starts_with("gh "))
    }

    #[must_use]
    /// True when an operation needs refusal or own-branch ownership.
    pub fn needs_core(&self) -> bool {
        self.ops
            .iter()
            .any(|o| matches!(o.kind, GitKind::Deny | GitKind::Own | GitKind::Credential))
    }

    #[must_use]
    /// True for a credential command.
    pub fn credential_read(&self) -> bool {
        self.ops.iter().any(|o| o.kind == GitKind::Credential)
    }

    #[must_use]
    /// True for a push or forge write.
    pub fn denied(&self) -> bool {
        self.ops.iter().any(|o| o.kind == GitKind::Deny)
    }
}

/// Read command starts after transparent wrappers and environment assignments, in each of the shell readings
/// (module doc); the result holds every operation any reading recognized, once.
#[must_use]
pub fn scan(command: &str) -> GitScan {
    let mut out = GitScan::default();
    for reading in readings(command) {
        let one = scan_reading(
            reading
                .iter()
                .map(|segment| segment.iter().map(String::as_str).collect()),
        );
        out.mentions |= one.mentions;
        for op in one.ops {
            if !out.ops.contains(&op) {
                out.ops.push(op);
            }
        }
    }
    out
}

/// Unquoted literal words, plus the dequoted words as Bash and Windows shells read them.
pub(crate) fn readings(command: &str) -> [Vec<Vec<String>>; 3] {
    let dequoted = |parsed: Vec<crate::killparse_shell::Segment>| {
        parsed
            .into_iter()
            .map(|segment| segment.words.into_iter().map(|word| word.text).collect())
            .collect()
    };
    [
        segments(command)
            .into_iter()
            // A literal reading cannot resolve quoted arguments; the shell readings below can.
            .filter(|segment| !segment.iter().any(|word| word.contains(['"', '\''])))
            .map(|segment| segment.into_iter().map(str::to_owned).collect())
            .collect(),
        dequoted(crate::killparse_shell::segments(command)),
        dequoted(crate::killparse_shell::windows_segments(command)),
    ]
}

fn scan_reading<'a>(reading: impl Iterator<Item = Vec<&'a str>>) -> GitScan {
    let mut out = GitScan::default();
    let mut elsewhere = false;
    for segment in reading {
        let words = command_words(&segment);
        let Some((&name, args)) = words.split_first() else {
            continue;
        };
        if matches!(name, "cd" | "pushd" | "popd") {
            elsewhere = true;
            continue;
        }
        let name = executable_name(name).to_ascii_lowercase();
        let mut switched = false;
        if name == "git" {
            out.mentions = true;
            crate::gitparse_cmds::git(args, elsewhere, &mut switched, Sink(&mut out));
        } else if name == "gh" {
            out.mentions = true;
            crate::gitparse_cmds::gh(args, Sink(&mut out));
        } else if let Some(sub) = name.strip_prefix("git-") {
            out.mentions = true;
            let mut all = vec![sub];
            all.extend_from_slice(args);
            crate::gitparse_cmds::git(&all, elsewhere, &mut switched, Sink(&mut out));
        }
        elsewhere |= switched;
    }
    out
}

pub(crate) struct Sink<'a>(&'a mut GitScan);

impl Sink<'_> {
    pub(crate) fn op(&mut self, kind: GitKind, what: impl Into<String>, elsewhere: bool) {
        self.0.ops.push(GitOp {
            kind,
            what: what.into(),
            elsewhere,
            name: None,
            rename_to: None,
            git_dir: None,
            work_tree: None,
        });
    }

    pub(crate) fn named(
        &mut self,
        kind: GitKind,
        what: impl Into<String>,
        elsewhere: bool,
        name: &str,
        _rest: &[&str],
    ) {
        self.0.ops.push(GitOp {
            kind,
            what: what.into(),
            elsewhere,
            name: Some(name.to_owned()),
            rename_to: None,
            git_dir: None,
            work_tree: None,
        });
    }

    pub(crate) fn rename(&mut self, elsewhere: bool, from: Option<&str>, to: &str) {
        self.0.ops.push(GitOp {
            kind: GitKind::Own,
            what: "git branch (move)".to_owned(),
            elsewhere,
            name: from.map(str::to_owned),
            rename_to: Some(to.to_owned()),
            git_dir: None,
            work_tree: None,
        });
    }

    pub(crate) fn len(&self) -> usize {
        self.0.ops.len()
    }

    pub(crate) fn paths(&mut self, start: usize, git_dir: Option<&str>, work_tree: Option<&str>) {
        for op in &mut self.0.ops[start..] {
            op.git_dir = git_dir.map(str::to_owned);
            op.work_tree = work_tree.map(str::to_owned);
        }
    }
}
