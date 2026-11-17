//! Checkout metadata tests for PR head identity; uses disposable Git configuration and no network.

use super::head_for_branch;
use crate::forge::parse_remote;
use std::path::Path;

fn git(root: &Path, args: &[&str]) {
    assert!(
        std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn checkout_metadata_keeps_the_fork_owner_and_remote_branch() {
    let root = std::env::temp_dir().join(format!("vw-pr-head-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    vw_changes::prepare_root(&root.join("changes")).unwrap();
    git(&root, &["init", "-q"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/upstream/project.git",
        ],
    );
    git(
        &root,
        &[
            "remote",
            "add",
            "contributor",
            "git@github.com:contributor/project.git",
        ],
    );
    let forge = parse_remote("https://github.com/upstream/project").unwrap();
    assert_eq!(
        head_for_branch(&root, &forge, "local-alias").unwrap(),
        "local-alias"
    );
    for (remote, merge, expected) in [
        (
            "https://github.com/contributor/project.git",
            "refs/heads/topic",
            "contributor:topic",
        ),
        ("contributor", "refs/heads/topic", "contributor:topic"),
        ("origin", "refs/heads/topic", "topic"),
        ("origin", "refs/pull/17/head", "17"),
        ("origin", "refs/pull/not-a-number/head", "local-alias"),
        ("origin", "refs/pull/0/head", "local-alias"),
        ("origin", "refs/tags/topic", "local-alias"),
        ("origin", "refs/heads/--bad", "local-alias"),
        (
            "https://gitlab.com/contributor/project",
            "refs/heads/topic",
            "local-alias",
        ),
        (
            "ext::echo should-not-run",
            "refs/heads/topic",
            "local-alias",
        ),
        ("missing", "refs/heads/topic", "local-alias"),
    ] {
        git(&root, &["config", "branch.local-alias.remote", remote]);
        git(&root, &["config", "branch.local-alias.merge", merge]);
        assert_eq!(
            head_for_branch(&root, &forge, "local-alias").unwrap(),
            expected,
            "{remote} {merge}"
        );
    }
    git(&root, &["config", "--unset", "branch.local-alias.remote"]);
    git(
        &root,
        &["config", "branch.local-alias.merge", "refs/heads/topic"],
    );
    assert_eq!(
        head_for_branch(&root, &forge, "local-alias").unwrap(),
        "local-alias"
    );
    let _ = std::fs::remove_dir_all(root);
}
