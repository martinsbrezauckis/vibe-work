//! Project identity when git is not installed ([`super::capture_with`] with a stand-in for git).
//!
//! PREDICT (written before the change): every error of the repository reader refused the identity, so on Windows
//! without Git, Settings > Project secrets showed "the project's worktree identity could not be checked", no entries
//! and a disabled "+ Add entry" (with Git installed the same page worked). Git that cannot be started now means no git
//! identity: the folder is its own project and never inherits; a linked copy whose link cannot be checked says so;
//! another user's repository or a git timeout still refuses.

use std::collections::BTreeSet;
use std::path::PathBuf;

use vw_changes::ChangesError;
use vw_worktrees::WorktreeError;

fn missing(
    _: &std::path::Path,
) -> Result<Option<vw_worktrees::repo_info::RepoInfo>, WorktreeError> {
    Err(WorktreeError::Git(ChangesError::GitMissing(
        "No such file or directory (os error 2)".to_owned(),
    )))
}

#[test]
fn without_git_a_plain_folder_is_its_own_project() {
    let dir = tempfile::tempdir().unwrap();
    let identity = super::capture_with(dir.path().to_str().unwrap(), &BTreeSet::new(), missing)
        .expect("git that is not installed is no refusal");
    assert_eq!(identity.main, None);
    assert!(!identity.inherit);
    assert!(!identity.link_unverified, "no .git file: nothing to verify");
    assert_eq!(identity.allowed_project(), identity.plain);
}

#[test]
fn without_git_a_linked_copy_never_inherits_and_says_its_link_is_unchecked() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    let copy = dir.path().join("copy");
    std::fs::create_dir_all(main.join(".git/worktrees/copy")).unwrap();
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join(".git"), "gitdir: ../main/.git/worktrees/copy\n").unwrap();
    // The main project is a registered project: with git, a verified link would inherit its entries.
    let registered: BTreeSet<PathBuf> = [vw_platform::canonical_plain(&main).unwrap()].into();
    let identity = super::capture_with(copy.to_str().unwrap(), &registered, missing).unwrap();
    assert_eq!(identity.main, None);
    assert!(
        !identity.inherit,
        "an unchecked link never grants the main project's entries"
    );
    assert!(identity.link_unverified);
    assert_eq!(identity.allowed_project(), identity.plain);
    // The main project it names is read deny-only: a Never there still refuses in the copy.
    assert_eq!(
        identity.denies,
        vec![
            vw_platform::canonical_plain(&main)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ]
    );
}

#[test]
fn without_git_a_link_of_another_shape_adds_no_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("copy");
    std::fs::create_dir_all(dir.path().join("elsewhere")).unwrap();
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join(".git"), "gitdir: ../elsewhere\n").unwrap();
    let identity = super::capture_with(copy.to_str().unwrap(), &BTreeSet::new(), missing).unwrap();
    assert!(identity.link_unverified);
    assert_eq!(identity.denies, Vec::<String>::new());
    assert!(!identity.inherit);
}

#[test]
fn a_real_git_failure_still_refuses() {
    let dir = tempfile::tempdir().unwrap();
    for error in [
        WorktreeError::NotGit("git refuses this folder: it belongs to another user".to_owned()),
        WorktreeError::Git(ChangesError::Timeout {
            command: "rev-parse".to_owned(),
            seconds: 10,
        }),
    ] {
        let refused = super::capture_with(dir.path().to_str().unwrap(), &BTreeSet::new(), |_| {
            Err(error)
        })
        .unwrap_err();
        assert_eq!(
            refused,
            "the project's worktree identity could not be checked"
        );
    }
}
