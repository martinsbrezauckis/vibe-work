//! Throwaway linked worktrees support inspection tests.

#![allow(dead_code, unreachable_pub)]

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// What the user sees of their main checkout. Equal before and after = untouched.
#[derive(Debug, PartialEq, Eq)]
pub struct Checkout {
    pub status: String,
    pub index_sha256: String,
    pub head: String,
    pub head_commit: String,
    pub stash: String,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Reads the main checkout's state (status with the branch line, index bytes hash, `HEAD` file, its commit,
/// stash list). Uses plain git with optional locks off, so reading changes nothing.
pub fn checkout(root: &Path) -> Checkout {
    let index = std::fs::read(root.join(".git/index")).unwrap_or_default();
    Checkout {
        status: git(
            root,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--untracked-files=all",
            ],
        ),
        index_sha256: format!("{:x}", Sha256::digest(&index)),
        head: std::fs::read_to_string(root.join(".git/HEAD")).unwrap(),
        head_commit: git(root, &["rev-parse", "HEAD"]),
        stash: git(root, &["stash", "list"]),
    }
}

/// A linked worktree at `<base>/wt/<name>` on a new branch `branch` from `HEAD` (what Vibe's `create` does
/// for a branch that exists: here the branch is made by `worktree add -b`, test setup only).
pub fn add_worktree(root: &Path, base: &Path, name: &str, branch: &str) -> PathBuf {
    let path = base.join("wt").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    git(
        root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            path.to_str().unwrap(),
        ],
    );
    path.canonicalize().unwrap()
}

/// Runs git in `dir` (test setup only).
pub fn run(dir: &Path, args: &[&str]) -> String {
    git(dir, args)
}
