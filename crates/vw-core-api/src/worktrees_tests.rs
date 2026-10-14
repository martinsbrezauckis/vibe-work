//! Unit tests of `worktrees.rs` (copy counting, managed copy branches, session lookups),
//! split out to keep the module under the 800-line limit. Called by: `cargo test` only.

use super::{count_copies, managed_copy_branch, with_session};
use std::path::{Path, PathBuf};
use std::process::Command;
use vw_worktrees::{Decision, Record, Records};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vw-core-copy-options-{}-{}",
            std::process::id(),
            vw_changes::now_ms()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn count_only_extant_recorded_linked_copies() {
    let scratch = std::env::temp_dir().join(format!(
        "vw-fx30-count-{}-{}",
        std::process::id(),
        vw_changes::now_ms()
    ));
    let repo = scratch.join("repo");
    let vibe_root = scratch.join("vibe/worktrees");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&vibe_root).unwrap();
    vw_changes::prepare_root(&scratch.join("changes")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("readme"), "base").unwrap();
    git(&repo, &["add", "readme"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "base",
        ],
    );
    let records = Records::open(&scratch.join("records")).unwrap();
    let add = |name: &str| {
        let path = vibe_root.join(name);
        git(
            &repo,
            &["worktree", "add", "-q", "-b", name, path.to_str().unwrap()],
        );
        path
    };
    let kept = add("kept");
    let orphan = add("orphan");
    let missing = add("missing");
    let removed = add("removed");
    let _unmanaged = add("unmanaged");
    let outside = scratch.join("outside");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "outside",
            outside.to_str().unwrap(),
        ],
    );
    for (name, decision, deleted) in [
        ("kept", Some(Decision::Kept), false),
        ("orphan", None, true),
        ("missing", None, false),
        ("removed", Some(Decision::Removed), false),
        ("outside", None, false),
        ("main", None, false),
    ] {
        records
            .put(&Record {
                session_id: name.into(),
                repo_root: repo.clone(),
                base_name: "main".into(),
                base_commit: "unused".into(),
                branch: name.into(),
                folder: name.into(),
                created_branch: true,
                created_at_ms: 1,
                decision,
                session_deleted: deleted,
                session_archived: false,
            })
            .unwrap();
    }
    git(&repo, &["worktree", "remove", missing.to_str().unwrap()]);
    assert_eq!(count_copies(&records, &vibe_root), 2);
    assert!(kept.is_dir() && orphan.is_dir() && removed.is_dir());
    std::fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn native_git_action_names_session_and_keeps_entire_summary() {
    let text = format!("Push {}?", "long-target/".repeat(200));
    let summary = with_session(&text, "s1", Some("Important session"));
    assert!(summary.starts_with("Session \"Important session\" (ID \"s1\")"));
    assert!(summary.ends_with(&text));
}

#[test]
fn recorded_copy_is_recognized_only_at_its_exact_live_folder() {
    let temp = TestDir::new();
    let repo = temp.0.join("repo");
    let vibe_root = temp.0.join("vibe-worktrees");
    let copy = vibe_root.join("repository").join("copy");
    vw_changes::prepare_root(&temp.0.join("changes")).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    git(
        &repo,
        &["worktree", "add", "-b", "vibe/copy", copy.to_str().unwrap()],
    );
    let records = Records::open(&temp.0.join("records")).unwrap();
    let mut record = Record {
        session_id: "session-one".into(),
        repo_root: repo.clone(),
        base_name: "main".into(),
        base_commit: String::new(),
        branch: "vibe/copy".into(),
        folder: "copy".into(),
        created_branch: true,
        created_at_ms: 1,
        decision: None,
        session_deleted: true,
        session_archived: false,
    };
    records.put(&record).unwrap();
    assert_eq!(records.all().len(), 1);
    // git reports the worktree path resolved and with forward slashes (macOS /private/var, Windows long names
    // instead of RUNNER~1, snapshot 9 CI): compare both sides resolved.
    assert_eq!(
        super::find_path(&repo, "vibe/copy").map(|path| path.canonicalize().unwrap()),
        Some(copy.canonicalize().unwrap()),
        "{:?}",
        vw_worktrees::repo_info::worktrees(&repo)
    );
    assert_eq!(
        managed_copy_branch(&records, &vibe_root, &copy).as_deref(),
        Some("vibe/copy")
    );
    assert_eq!(managed_copy_branch(&records, &vibe_root, &repo), None);
    std::fs::create_dir(copy.join("nested")).unwrap();
    assert_eq!(
        managed_copy_branch(&records, &vibe_root, &copy.join("nested")),
        None
    );
    record.decision = Some(Decision::Removed);
    records.put(&record).unwrap();
    assert_eq!(managed_copy_branch(&records, &vibe_root, &copy), None);
}
