//! Per-subcommand readers of the git guard (`gitparse`): git's global options and every subcommand's rule,
//! and GitHub's CLI. The policy is in `gitparse`'s module docs; this file applies it. Unknown git
//! subcommands are outside this guardrail; unknown gh commands are refused.
//!
//! Depends on: `gitparse` (`GitKind`, `Sink`), `branch` (checkout, switch, reset, update-ref and refspec readers).
//! Called by: `gitparse::scan`.

use crate::gitparse::{GitKind, Sink};

mod branch;

use branch::{
    BranchEffect, branch_name, checkout, named_branch, refspec_targets, reset, switch,
    switch_flags, update_ref,
};

/// Reads one `git` command (`args` after `git`). Returns true when a global option (`-C`, `--git-dir`,
/// `--work-tree`, `--namespace`) points it at another repository or folder.
pub(crate) fn git(args: &[&str], elsewhere: bool, switched: &mut bool, mut sink: Sink<'_>) -> bool {
    let mut away = elsewhere;
    let mut flagged = false;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match *a {
            // `-C .` is this folder, not another repository; holding it made an own-copy
            // `git -C . rebase main` wait for the user. Any other `-C` value still counts as elsewhere.
            "-C" if args
                .get(i + 1)
                .is_some_and(|v| matches!(*v, "." | "./" | ".\\")) =>
            {
                i += 2;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" => {
                away = true;
                flagged = true;
                i += 2;
            }
            "-c" | "--config-env" => i += 2,
            _ if a.starts_with("-c") || a.starts_with("--config-env=") => i += 1,
            "--version" | "--help" | "-h" => return flagged,
            _ if a.starts_with("-C")
                || a.starts_with("--git-dir=")
                || a.starts_with("--work-tree=") =>
            {
                away = true;
                flagged = true;
                i += 1;
            }
            _ if a.starts_with("--namespace=") => {
                away = true;
                flagged = true;
                i += 1;
            }
            _ if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let Some(sub) = args.get(i).copied() else {
        return flagged;
    };
    let rest = &args[i + 1..];
    subcommand(sub, rest, away, switched, &mut sink);
    flagged
}

fn has(rest: &[&str], flags: &[&str]) -> bool {
    rest.iter().any(|a| flags.contains(a))
}

/// A cluster of short flags (`-df`) contains one of `letters`.
fn short_flag(rest: &[&str], letters: &str) -> bool {
    rest.iter().any(|a| {
        a.len() > 1
            && a.starts_with('-')
            && !a.starts_with("--")
            && a[1..].chars().all(|c| c.is_ascii_alphabetic())
            && a[1..].chars().any(|c| letters.contains(c))
    })
}

fn positionals<'a>(rest: &[&'a str], with_value: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip = false;
    let mut options = true;
    for a in rest {
        if skip {
            skip = false;
            continue;
        }
        if options && *a == "--" {
            options = false;
            continue;
        }
        if options && a.starts_with('-') {
            skip = with_value.contains(a);
            continue;
        }
        out.push(*a);
    }
    out
}

#[allow(clippy::too_many_lines)] // one match over git's subcommands reads best as one table
fn subcommand(sub: &str, rest: &[&str], away: bool, switched: &mut bool, sink: &mut Sink<'_>) {
    let first = rest.first().copied();
    match sub {
        "push" | "send-pack" | "http-push" => sink.op(GitKind::Deny, format!("git {sub}"), away),
        // A `src:dst` refspec writes the local branch `dst` (`git fetch
        // origin main:main`): a rewrite of that branch, held unless it is the session's own.
        "fetch" | "pull" => {
            sink.op(GitKind::Change, format!("git {sub}"), away);
            refspec_targets(rest, away, sink);
        }
        "lfs" if first == Some("push") => sink.op(GitKind::Deny, "git lfs push", away),
        "reset" => reset(rest, away, sink),
        "branch" => {
            let changes_others = has(rest, &["--delete", "--move", "--copy", "--force"])
                || short_flag(rest, "dDmMcCf");
            if changes_others {
                // Each branch it deletes, moves, copies onto or forces is
                // named, so the core holds it unless it is the session's own branch (`git branch -m new` renames the
                // current branch to `new`: a name that is not the own one is held).
                let all = positionals(rest, &["-u", "--set-upstream-to"]);
                // Delete and move change every name given; copy writes the last one (`-c old new`, `-c new`); force
                // alone writes the first (`git branch -f <b> <start>`: the start point is not changed).
                let names: Vec<&str> =
                    if has(rest, &["--delete", "--move"]) || short_flag(rest, "dDmM") {
                        all
                    } else if has(rest, &["--copy"]) || short_flag(rest, "cC") {
                        all.last().copied().into_iter().collect()
                    } else {
                        all.first().copied().into_iter().collect()
                    };
                let what = "git branch (delete, move, copy or force)";
                if names.is_empty() {
                    sink.op(GitKind::Own, what, away);
                }
                for name in names {
                    sink.named(GitKind::Own, what, away, branch_name(name), &[]);
                }
            }
        }
        "stash" => {
            if first == Some("branch") {
                // `git stash branch <name>` creates that branch and switches to it.
                *switched = true;
            }
            if matches!(first, Some("drop" | "clear")) {
                // Stashes are one list shared by every copy of the repository, so a drop
                // or clear can delete the user's saved work even from the agent's own copy. Held always: the
                // target is never the agent's branch.
                sink.named(
                    GitKind::Own,
                    "git stash drop/clear (deletes saved stashes; every copy shares them)",
                    away,
                    "refs/stash",
                    &[],
                );
            } else if !matches!(first, Some("list" | "show")) {
                sink.op(GitKind::Change, "git stash", away);
            }
        }
        "worktree" => {
            if matches!(first, Some("remove" | "move")) {
                // Removing or moving a copy (worktree) can delete or strand someone else's work; Vibe
                // Work removes its own copies after the user saw the file list. Held always.
                sink.named(
                    GitKind::Own,
                    format!(
                        "git worktree {} (removes or moves a copy)",
                        first.unwrap_or("")
                    ),
                    away,
                    "a worktree",
                    &[],
                );
            } else if first != Some("list") {
                sink.op(
                    GitKind::Change,
                    format!("git worktree {}", first.unwrap_or("")),
                    away,
                );
            }
            // `git worktree add -B <branch>` resets that branch.
            if first == Some("add") {
                let flags = switch_flags(&rest[1..], 'b', 'B', false);
                if flags.effect == BranchEffect::Resets {
                    named_branch(
                        sink,
                        "git worktree add -B (resets a branch)",
                        away,
                        flags.value.as_deref(),
                    );
                }
            }
        }
        "rebase" => {
            let pos = positionals(
                rest,
                &[
                    "--onto",
                    "-C",
                    "--empty",
                    "--whitespace",
                    "-s",
                    "--strategy",
                    "-X",
                    "--strategy-option",
                    "-x",
                    "--exec",
                ],
            );
            // `git rebase <upstream> <branch>` (or `--root <branch>`) first switches to `<branch>` and rewrites it.
            let target = if has(rest, &["--root"]) {
                pos.first()
            } else {
                pos.get(1)
            };
            // a rebase rewrites a branch; the branch it names
            // is kept, so a rebase of another branch is held even from the own copy branch.
            match target {
                Some(branch) => {
                    *switched = true;
                    sink.named(GitKind::Own, "git rebase", away, branch_name(branch), &[]);
                }
                None => sink.op(GitKind::Own, "git rebase", away),
            }
        }
        "checkout" => checkout(rest, away, switched, sink),
        "switch" => switch(rest, away, switched, sink),
        "update-ref" => update_ref(rest, away, sink),
        "symbolic-ref" => {
            let pos = positionals(rest, &["-m"]);
            if pos.len() >= 2 || has(rest, &["-d", "--delete"]) {
                if pos.first() == Some(&"HEAD") {
                    // `git symbolic-ref HEAD refs/heads/<b>` switches the branch.
                    *switched = true;
                }
                // Pointing HEAD (or a branch) elsewhere without updating the files leaves
                // a folder half-switched; the agent has `git switch` for ordinary use. Held always.
                sink.named(
                    GitKind::Own,
                    "git symbolic-ref (points HEAD or a branch elsewhere)",
                    away,
                    "a symbolic ref",
                    &[],
                );
            }
        }
        "submodule" => sink.op(GitKind::Change, "git submodule", away),
        // These ordinary local changes need no special reader or dialog.
        "commit" | "cherry-pick" | "revert" | "am" | "merge" | "remote" | "lfs" | "reflog"
        | "tag" | "clean" | "filter-branch" | "filter-repo" | "fast-import" | "receive-pack"
        | "replace" => sink.op(GitKind::Change, format!("git {sub}"), away),
        "config" => sink.op(GitKind::Change, "git config", away),
        // The user's stored credentials must never reach the agent.
        _ if sub.starts_with("credential") => {
            sink.op(
                GitKind::Credential,
                format!("git {sub} (your stored git credentials)"),
                away,
            );
        }
        _ if sub.starts_with('-') => {}
        _ => {} // Unknown command names and aliases are outside the pure guardrail.
    }
}

/// Every `gh` command is refused for the agent: GitHub data comes through
/// Mistral's GitHub connector. Commands that would print the user's token keep their own credential refusal. The
/// former read allowance for commands such as `gh pr view` is removed; any future broker must read through
/// the user's login without exposing credentials to the agent.
pub(crate) fn gh(args: &[&str], mut sink: Sink<'_>) {
    let pos = positionals(args, &["-R", "--repo", "--hostname", "-h"]);
    let group = pos.first().copied().unwrap_or("");
    let action = pos.get(1).copied().unwrap_or("");
    // Fail closed so the user's GitHub token never reaches the agent.
    if group == "auth" {
        sink.op(
            GitKind::Credential,
            format!("gh auth {action} (prints your GitHub token)"),
            false,
        );
        return;
    }
    let what = match (group, action) {
        ("pr", "create") => "gh pr create (the Create PR button does this for the user)".to_owned(),
        // A bare `gh` or `gh --version` still names the CLI after "gh " (`GitScan::forge_write` matches that prefix).
        ("", _) => "gh (the GitHub CLI)".to_owned(),
        _ => format!("gh {group} {action}").trim().to_owned(),
    };
    sink.op(GitKind::Deny, what, false);
}
