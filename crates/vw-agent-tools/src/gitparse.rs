//! Classifies literal Git and GitHub CLI command starts for the hook and core.
//! Depends on `command_segments`, `killparse_shell` (quote-aware readings) and `gitparse_cmds`; called by the hook,
//! core and tests.
//! This is a guardrail: only shell `-c` payloads are also checked for credential calls; ordinary argument data is not code. Each command is read as unquoted literal words and as Bash and Windows
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
    /// Sets the Git identity (the name and email Git writes into commits): the user's to set, refused.
    Identity,
    /// A shell call whose command text cannot be read here (PowerShell reading standard input, for example):
    /// always held for the user.
    Unread,
}

/// Git config keys that make up the identity commits carry (compared in lower case).
pub(crate) const IDENTITY_KEYS: [&str; 6] = [
    "user.name",
    "user.email",
    "author.name",
    "author.email",
    "committer.name",
    "committer.email",
];

/// Environment variables that set the identity of a commit (compared in upper case).
const IDENTITY_VARIABLES: [&str; 4] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
];

/// True when `key` is one of [`IDENTITY_KEYS`].
pub(crate) fn identity_key(key: &str) -> bool {
    IDENTITY_KEYS.contains(&key.to_ascii_lowercase().as_str())
}

/// True when the segment assigns one of [`IDENTITY_VARIABLES`]: as leading assignments (`NAME=x git ...`), after
/// `export`/`declare`/`typeset`/`readonly`/`env`, with `setx NAME value`, or as PowerShell's `$env:NAME=...`.
fn assigns_identity(segment: &[&str]) -> bool {
    let named = |word: &str| {
        let word = word
            .get(..5)
            .filter(|p| p.eq_ignore_ascii_case("$env:"))
            .map_or(word, |_| &word[5..]);
        let name = word.split_once('=').map_or("", |(name, _)| name);
        IDENTITY_VARIABLES.contains(&name.to_ascii_uppercase().as_str())
    };
    let Some((&first, rest)) = segment.split_first() else {
        return false;
    };
    match first.to_ascii_lowercase().as_str() {
        "export" | "declare" | "typeset" | "readonly" | "env" => rest.iter().any(|w| named(w)),
        "setx" => rest
            .first()
            .is_some_and(|w| IDENTITY_VARIABLES.contains(&w.to_ascii_uppercase().as_str())),
        _ => segment
            .iter()
            .take_while(|w| w.contains('='))
            .any(|w| named(w)),
    }
}

/// Reason for refusing credential commands.
pub const CREDENTIAL_REASON: &str = "Not run: the agent cannot use the user's stored Git or GitHub sign-in. Continue without reading credentials; the user can push or create a PR in the Changes tab.";
/// Reason for refusing a Git identity the agent sets itself.
pub const IDENTITY_REASON: &str = "Not run: the name and email Git writes into commits are the user's to set. When a commit needs them, Vibe Work asks the user.";
/// Reason for refusing a GitHub CLI command while reads are off.
pub const GH_WRITE_REASON: &str = vw_gh_broker::remote_change_refusal!(concat!(
    "GitHub reads are off. Ask the user to turn them on in ",
    vw_vocabulary::settings_path!("General"),
    ", or use the GitHub connector for reads."
));

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
    /// True when an operation needs refusal, own-branch ownership or the person's Git identity.
    pub fn needs_core(&self) -> bool {
        self.needs_identity()
            || self.ops.iter().any(|o| {
                matches!(
                    o.kind,
                    GitKind::Deny
                        | GitKind::Own
                        | GitKind::Credential
                        | GitKind::Identity
                        | GitKind::Unread
                )
            })
    }

    #[must_use]
    /// True for a `git init` or `git commit`: the core makes sure the person's name and email exist first (it asks
    /// the person when Git has none).
    pub fn needs_identity(&self) -> bool {
        self.ops
            .iter()
            .any(|o| o.what == "git init" || o.what == "git commit")
    }

    #[must_use]
    /// True when the command sets the Git identity ([`GitKind::Identity`]).
    pub fn identity_set(&self) -> bool {
        self.ops.iter().any(|o| o.kind == GitKind::Identity)
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
    let mut pending = vec![(command.to_owned(), false)];
    let mut seen = std::collections::BTreeSet::new();
    while let Some((command, credentials_only)) = pending.pop() {
        if !seen.insert(command.clone()) {
            continue;
        }
        for reading in readings(&command) {
            let one = scan_reading(
                reading
                    .iter()
                    .map(|segment| segment.iter().map(String::as_str).collect()),
                &mut pending,
            );
            out.mentions |= if credentials_only {
                one.credential_read()
            } else {
                one.mentions
            };
            for op in one.ops {
                if (!credentials_only || matches!(op.kind, GitKind::Credential | GitKind::Unread))
                    && !out.ops.contains(&op)
                {
                    out.ops.push(op);
                }
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

fn scan_reading<'a>(
    reading: impl Iterator<Item = Vec<&'a str>>,
    scripts: &mut Vec<(String, bool)>,
) -> GitScan {
    let mut out = GitScan::default();
    let mut elsewhere = false;
    for segment in reading {
        if assigns_identity(&segment) {
            out.mentions = true;
            Sink(&mut out).op(
                GitKind::Identity,
                "GIT_AUTHOR_* / GIT_COMMITTER_* (the user's Git identity)",
                elsewhere,
            );
        }
        let words = command_words(&segment);
        let Some((&name, args)) = words.split_first() else {
            continue;
        };
        if matches!(name, "cd" | "pushd" | "popd") {
            elsewhere = true;
            continue;
        }
        let name = executable_name(name).to_ascii_lowercase();
        if matches!(name.as_str(), "bash" | "sh") {
            let mut at = 0;
            while let Some(arg) = args.get(at) {
                if *arg == "--" || !arg.starts_with('-') {
                    break;
                }
                if matches!(*arg, "-o" | "-O") {
                    at += 2;
                    continue;
                }
                if !arg.starts_with("--") && arg.contains('c') {
                    if let Some(script) = args.get(at + 1) {
                        scripts.push(((*script).to_owned(), true));
                    }
                    break;
                }
                at += 1;
            }
        }
        // Windows shells: `cmd /c …` (Git Bash writes `cmd //c`) and PowerShell run command text, the natural Windows
        // form of a wrapped credential read. Text PowerShell takes from elsewhere is held for the user.
        let shell = crate::windows_shell::command_text(&name, args);
        for script in shell.scripts {
            scripts.push((script, true));
        }
        if let Some(why) = shell.unreadable {
            out.mentions = true;
            Sink(&mut out).op(GitKind::Unread, format!("PowerShell ({why})"), elsewhere);
        }
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
