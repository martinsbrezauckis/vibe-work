//! The git guard's readers of commands that switch, create, move or rewrite branches (`gitparse_cmds`, split off in
//! these include checkout, switch, reset, update-ref, `src:dst` refspecs, and the branch names they write, so the core
//! can hold a branch-rewriting op on a branch that is not the session's own.
//!
//! Depends on: `gitparse` (`GitKind`, `Sink`), the parent's flag helpers. Called by: `gitparse_cmds::subcommand`.

use super::{has, positionals};
use crate::gitparse::{GitKind, Sink};

/// `git update-ref`: `refs/heads/<b>` (also with `-d`) moves or deletes
/// that branch (named); `--stdin` can move any branch (named `*`: held everywhere); `HEAD`, `@`, a bare name or any
/// other ref that may be symbolic moves the current branch (unnamed: held off the own copy branch); refs that are
/// clearly not branches (`refs/tags/`, `refs/notes/`, `refs/remotes/`: remote-tracking refs are visible to the user
/// but make no commit on a branch; `refs/stash`) are a `Change`.
pub(super) fn update_ref(rest: &[&str], away: bool, sink: &mut Sink<'_>) {
    if has(rest, &["--stdin"]) {
        sink.named(
            GitKind::Own,
            "git update-ref --stdin (moves any branch)",
            away,
            "*",
            &[],
        );
        return;
    }
    let pos = positionals(rest, &["-m"]);
    let Some(target) = pos.first().copied() else {
        return;
    };
    if let Some(branch) = target.strip_prefix("refs/heads/") {
        sink.named(
            GitKind::Own,
            "git update-ref (moves a branch)",
            away,
            branch,
            &[],
        );
    } else if ["refs/tags/", "refs/notes/", "refs/remotes/"]
        .iter()
        .any(|p| target.starts_with(p))
        || target == "refs/stash"
    {
        sink.op(GitKind::Change, "git update-ref", away);
    } else {
        sink.op(
            GitKind::Own,
            "git update-ref (moves the current branch)",
            away,
        );
    }
}

/// The local branch a name stands for (`refs/heads/x` -> `x`; a leading `+` of a forced refspec dropped).
pub(super) fn branch_name(name: &str) -> &str {
    let name = name.trim_start_matches('+');
    name.strip_prefix("refs/heads/").unwrap_or(name)
}

/// A branch-rewriting op on `branch` (named, so the core can tell the session's own branch), or on an unnamed one.
pub(super) fn named_branch(sink: &mut Sink<'_>, what: &str, away: bool, branch: Option<&str>) {
    match branch {
        Some(b) => sink.named(GitKind::Own, what, away, branch_name(b), &[]),
        None => sink.op(GitKind::Own, what, away),
    }
}

/// `git fetch|pull <remote> <src>:<dst>`: each `dst` is a local branch written by the command.
pub(super) fn refspec_targets(rest: &[&str], away: bool, sink: &mut Sink<'_>) {
    let with_value = [
        "--depth",
        "--deepen",
        "--shallow-since",
        "--shallow-exclude",
        "-j",
        "--jobs",
    ];
    for spec in positionals(rest, &with_value) {
        if let Some((_, dst)) = spec.split_once(':')
            && !dst.is_empty()
        {
            sink.named(
                GitKind::Own,
                "git fetch into a local branch",
                away,
                branch_name(dst),
                &[],
            );
        }
    }
}

/// `git reset`: moving the branch to another commit (`git reset HEAD~1`, `git reset --hard origin/main`) is
/// branch-rewriting (the branch's history changes, like a merge); a reset that keeps the
/// branch where it is (`git reset`, `git reset HEAD`, `git reset --hard`, `git reset -p`, `git reset -- a.txt`) is a
/// `Change`. Only the first positional before `--` can select the reset revision; later words are pathspecs or data.
pub(super) fn reset(rest: &[&str], away: bool, sink: &mut Sink<'_>) {
    let hard = has(rest, &["--hard"]);
    let patch = has(rest, &["-p", "--patch"]);
    // After `--`, reset arguments are paths, never a branch revision.
    let before = &rest[..rest.iter().position(|a| *a == "--").unwrap_or(rest.len())];
    let target = positionals(before, &[]).into_iter().next();
    match (target, patch) {
        (Some(revision), false) if !matches!(revision, "HEAD" | "@") => sink.op(
            GitKind::Own,
            "git reset (moves the branch to another commit)",
            away,
        ),
        _ if hard => sink.op(GitKind::Change, "git reset --hard", away),
        _ => sink.op(GitKind::Change, "git reset", away),
    }
}

/// What the flags of a checkout or switch do to the branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum BranchEffect {
    /// No switch-capable flag.
    #[default]
    None,
    /// `--detach`, `-t`/`--track`, `-` (the previous branch), `switch -d`: a switch.
    Switches,
    /// `-b`, `--orphan`, `switch -c`/`--create`: a new branch, switched to.
    Creates,
    /// `-B`, `switch -C`/`--force-create`: a branch created or reset, switched to.
    Resets,
}

/// The flags of a checkout or switch, read before `--`.
#[derive(Default)]
pub(super) struct SwitchFlags {
    /// The strongest branch effect of any flag.
    pub(super) effect: BranchEffect,
    /// `-p`/`--patch`, `--pathspec-from-file`: files are restored.
    pub(super) restores: bool,
    /// The words that are neither flags nor flag values.
    pub(super) names: Vec<String>,
    /// The branch a create/reset flag names (`-B x`, `-bname`, `--force-create=x`).
    pub(super) value: Option<String>,
}

/// Reads checkout/switch flags before `--`. `create`/`reset` are the short letters that take a branch name as their
/// value (`-bname`, `-b name`, `-fbname`: git's short-option clusters); their value is never a positional.
pub(super) fn switch_flags(
    before: &[&str],
    create: char,
    reset: char,
    detach_short: bool,
) -> SwitchFlags {
    let mut out = SwitchFlags::default();
    let raise = |out: &mut SwitchFlags, to: BranchEffect| out.effect = out.effect.max(to);
    let mut i = 0;
    while let Some(a) = before.get(i).copied() {
        i += 1;
        match a {
            "-" | "--detach" | "--track" => raise(&mut out, BranchEffect::Switches),
            "--orphan" | "--create" => {
                raise(&mut out, BranchEffect::Creates);
                out.value = before.get(i).map(|v| (*v).to_owned());
                i += 1; // the value
            }
            "--force-create" => {
                raise(&mut out, BranchEffect::Resets);
                out.value = before.get(i).map(|v| (*v).to_owned());
                i += 1;
            }
            "--patch" => out.restores = true,
            _ if a.starts_with("--orphan=") || a.starts_with("--create=") => {
                raise(&mut out, BranchEffect::Creates);
                out.value = a.split_once('=').map(|(_, v)| v.to_owned());
            }
            _ if a.starts_with("--force-create=") => {
                raise(&mut out, BranchEffect::Resets);
                out.value = a.split_once('=').map(|(_, v)| v.to_owned());
            }
            _ if a.starts_with("--track=") => raise(&mut out, BranchEffect::Switches),
            _ if a.starts_with("--pathspec-from-file") => out.restores = true,
            _ if a.starts_with("--") => {}
            _ if a.starts_with('-') => {
                for (at, c) in a.char_indices().skip(1) {
                    if c == create || c == reset {
                        let effect = if c == reset {
                            BranchEffect::Resets
                        } else {
                            BranchEffect::Creates
                        };
                        raise(&mut out, effect);
                        let attached = &a[at + c.len_utf8()..];
                        if attached.is_empty() {
                            out.value = before.get(i).map(|v| (*v).to_owned());
                            i += 1; // `-b name`: the next word is the value
                        } else {
                            out.value = Some(attached.to_owned());
                        }
                        break; // `-bname`: the rest of the cluster is the value
                    }
                    match c {
                        't' => raise(&mut out, BranchEffect::Switches),
                        'd' if detach_short => raise(&mut out, BranchEffect::Switches),
                        'p' => out.restores = true,
                        _ => {}
                    }
                }
            }
            _ => out.names.push(a.to_owned()),
        }
    }
    out
}

/// `git checkout`: a branch switch (or a new or reset branch), or a file restore. File restores are changes;
/// every branch-creating or switch-capable flag makes it a switch and its
/// value is not a positional; a restore needs `--` followed by a pathspec, or names that can only be paths, and no
/// switch-capable flag. All of them are `Change` (Vibe's card asks); a switch matters for a rewrite later in the same
/// command (`gitparse::scan`).
pub(super) fn checkout(rest: &[&str], away: bool, switched: &mut bool, sink: &mut Sink<'_>) {
    let dashdash = rest.iter().position(|a| *a == "--");
    let before = &rest[..dashdash.unwrap_or(rest.len())];
    let flags = switch_flags(before, 'b', 'B', false);
    // At least one pathspec after `--`: git restores files. `git checkout main --` (nothing after `--`) is still a
    // branch switch: git reads `--` with no pathspec as "the word before is a revision".
    let paths_after = dashdash.is_some_and(|at| at + 1 < rest.len());
    // Without `--`, words that can only be paths (`.`, `./x`, `../x`, `x/`, `:/x`), or two or more words
    // (`<commit> <paths>` or several paths: never one branch), restore files too (for example, `git checkout .`).
    let paths_only = flags.names.len() >= 2
        || flags.names.first().is_some_and(|n| {
            n == "."
                || n == ".."
                || n.starts_with("./")
                || n.starts_with("../")
                || n.starts_with(":/")
                || n.ends_with('/')
        });
    if flags.effect == BranchEffect::Resets {
        // `-B <b>` resets an existing branch `<b>`: branch-rewriting on that branch.
        *switched = true;
        named_branch(
            sink,
            "git checkout -B (creates or resets a branch and switches to it)",
            away,
            flags.value.as_deref(),
        );
    } else if flags.effect != BranchEffect::None {
        *switched = true;
        let what = if flags.effect == BranchEffect::Creates {
            "git checkout (a new branch)"
        } else {
            "git checkout (switches to a branch or a commit)"
        };
        sink.op(GitKind::Change, what, away);
    } else if flags.restores || paths_after || paths_only {
        sink.op(
            GitKind::Change,
            "git checkout of files (restores files, their uncommitted edits are lost; no branch switch)",
            away,
        );
    } else if !flags.names.is_empty() {
        // One name: git itself decides whether it is a branch or a file.
        *switched = true;
        sink.op(
            GitKind::Change,
            "git checkout (switches to a branch, or restores the files of that name)",
            away,
        );
    }
}

/// `git switch`: always a switch (`-c`/`--create` creates a branch, `-C`/`--force-create` creates or
/// resets one, `-d`/`--detach` a commit).
pub(super) fn switch(rest: &[&str], away: bool, switched: &mut bool, sink: &mut Sink<'_>) {
    if has(rest, &["-h", "--help"]) {
        return;
    }
    let flags = switch_flags(rest, 'c', 'C', true);
    *switched = true;
    match flags.effect {
        // `-C <b>` resets an existing branch `<b>`.
        BranchEffect::Resets => named_branch(
            sink,
            "git switch -C (creates or resets a branch and switches to it)",
            away,
            flags.value.as_deref(),
        ),
        BranchEffect::Creates => sink.op(GitKind::Change, "git switch (a new branch)", away),
        _ => sink.op(GitKind::Change, "git switch", away),
    }
}
