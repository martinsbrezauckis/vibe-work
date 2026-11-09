//! Counts real Git starts behind an isolated shim. Called by repo unit tests; git init must invalidate a negative.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::discover;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn non_repositories_stop_starting_git_until_a_marker_appears() {
    const FIXTURE: &str = "VW_GIT_DISCOVERY_FIXTURE";
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = PathBuf::from(root);
        replay(&root);
        return;
    }
    // This test needs genuinely marker-free ancestors: a ceiling passed only to Git would
    // still leave the cache seeing an enclosing .git. VW_TEST_ROOT and TMPDIR may be in a repo.
    let base = PathBuf::from("/tmp").canonicalize().unwrap();
    for parent in base.ancestors() {
        assert!(
            matches!(parent.join(".git").symlink_metadata(), Err(error)
            if error.kind() == std::io::ErrorKind::NotFound),
            "the discovery fixture needs non-repository ancestors: {}",
            parent.display()
        );
    }
    let root = base.join(format!("git-discovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    let home = root.join("home");
    for path in [&bin, &home, &root.join("plain")] {
        std::fs::create_dir_all(path).unwrap();
    }
    let log = root.join("calls");
    let quoted = log.to_string_lossy().replace('\'', "'\\''");
    let source = root.join("git-shim");
    std::fs::write(
        &source,
        format!("#!/bin/sh\nprintf 'git\\n' >> '{quoted}'\nexec /usr/bin/git \"$@\"\n"),
    )
    .unwrap();
    // A separate installer never leaves a writable executable fd in concurrently forked test children.
    assert!(
        Command::new("/usr/bin/install")
            .args(["-m", "0755"])
            .arg(source)
            .arg(bin.join("git"))
            .status()
            .unwrap()
            .success()
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "repo::discovery_tests::non_repositories_stop_starting_git_until_a_marker_appears",
            "--nocapture",
        ])
        .env(FIXTURE, &root)
        .env("PATH", &bin)
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn replay(root: &Path) {
    crate::prepare_root(&root.join("data")).unwrap();
    let folder = root.join("plain");
    for _ in 0..5 {
        assert_eq!(discover(&folder).unwrap(), None);
    }
    let calls = || {
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count()
    };
    assert_eq!(calls(), 1, "non-repository polling keeps starting git");
    let status = Command::new("/usr/bin/git")
        .args(["init", "-q"])
        .arg(&folder)
        .status()
        .unwrap();
    assert!(status.success());
    let found = discover(&folder)
        .unwrap()
        .expect("git init must be discovered");
    assert_eq!(found.root, folder);
    assert!(calls() > 1, "new .git must run real discovery");
    // Also discover a newly initialized parent of a previously checked child.
    let parent = root.join("parent");
    let child = parent.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&child, &alias).unwrap();
    assert_eq!(discover(&child).unwrap(), None);
    assert_eq!(discover(&alias).unwrap(), None);
    assert!(
        Command::new("/usr/bin/git")
            .args(["init", "-q"])
            .arg(&parent)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(discover(&child).unwrap().unwrap().root, parent);
    assert_eq!(discover(&alias).unwrap().unwrap().root, parent);
    let missing = root.join("missing-git");
    std::fs::create_dir(&missing).unwrap();
    let git = root.join("bin/git");
    let hidden = root.join("bin/hidden-git");
    std::fs::rename(&git, &hidden).unwrap();
    assert!(matches!(
        discover(&missing),
        Err(crate::ChangesError::GitMissing(_))
    ));
    std::fs::rename(&hidden, &git).unwrap();
    assert_eq!(
        discover(&missing).unwrap(),
        None,
        "errors must not be cached"
    );
}
