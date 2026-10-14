//! Temporary repositories verify worktree inspection and cleanup.

mod changes;
mod worktrees;

use changes::TempRepo;
use vw_worktrees::{Head, LostKind, branch, inspect, naming, read};
use worktrees::{add_worktree, checkout, run};

fn scan(repo: &TempRepo) -> std::path::PathBuf {
    repo.base.join("scan")
}

#[test]
fn repository_facts_and_the_preflight() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    repo.git(&["branch", "feature"]);
    repo.git(&["branch", "zz-last"]);
    let info = read(&repo.root).unwrap().expect("a repository");
    assert_eq!(
        info.head,
        Head::Branch {
            name: "main".into()
        }
    );
    let names: Vec<&str> = info.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["main", "feature", "zz-last"]);
    assert!(info.branches[0].current && !info.branches[1].current);
    assert!(info.not_offered.is_none(), "{:?}", info.not_offered);
    assert_eq!(info.worktrees.len(), 1);
    // A subfolder reports the same repository.
    std::fs::create_dir_all(repo.path("sub")).unwrap();
    assert_eq!(read(&repo.path("sub")).unwrap().unwrap().root, repo.root);

    // Detached.
    let commit = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    repo.git(&["checkout", "-q", "--detach"]);
    assert_eq!(
        read(&repo.root).unwrap().unwrap().head,
        Head::Detached { commit }
    );
    repo.git(&["checkout", "-q", "main"]);

    // Not git, and unborn.
    let plain = repo.base.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(read(&plain).unwrap().is_none());
    let unborn = TempRepo::with_files(&[]);
    let info = read(&unborn.root).unwrap().unwrap();
    assert_eq!(info.head, Head::Unborn);
    assert!(info.not_offered.unwrap().contains("no commit yet"));
}

#[test]
fn repository_programs_are_found_by_scope() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let offered = |repo: &TempRepo| read(&repo.root).unwrap().unwrap().not_offered;
    repo.git(&["config", "core.fsmonitor", "true"]);
    assert!(
        offered(&repo).is_none(),
        "the builtin fsmonitor runs no program"
    );
    repo.git(&["config", "core.fsmonitor", "/tmp/evil-monitor"]);
    assert!(offered(&repo).unwrap().contains("core.fsmonitor"));
    repo.git(&["config", "--unset", "core.fsmonitor"]);
    // The canonical git-lfs install.
    repo.git(&["config", "filter.lfs.clean", "git-lfs clean -- %f"]);
    repo.git(&["config", "filter.lfs.smudge", "git-lfs smudge -- %f"]);
    repo.git(&["config", "filter.lfs.process", "git-lfs filter-process"]);
    assert!(offered(&repo).is_none());
    // The same driver name with another command is a program of the repository's choosing.
    repo.git(&["config", "filter.lfs.smudge", "sh -c 'touch /tmp/x'"]);
    assert!(offered(&repo).unwrap().contains("filter.lfs.smudge"));
    repo.git(&["config", "filter.lfs.smudge", "git-lfs smudge -- %f"]);
    // An included file is the repository's own config too.
    let included = repo.base.join("inc.cfg");
    std::fs::write(&included, "[filter \"evil\"]\n\tprocess = /tmp/evil\n").unwrap();
    repo.git(&["config", "include.path", included.to_str().unwrap()]);
    assert!(offered(&repo).unwrap().contains("filter.evil.process"));
}

#[test]
fn the_session_branch_is_create_only_and_deleted_only_untouched() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let root = &repo.root;
    let base = vw_worktrees::repo_info::resolve_base(root, "main").unwrap();
    for bad in ["-x", "a..b", "x.lock", "has space", "a\u{7}b", "@{-1}", ""] {
        assert!(
            branch::create(root, bad, &base).is_err(),
            "{bad:?} accepted"
        );
    }
    assert!(vw_worktrees::repo_info::resolve_base(root, "nope").is_err());
    assert!(vw_worktrees::repo_info::resolve_base(root, "--all").is_err());
    let before = repo.git(&["for-each-ref", "--format=%(refname)"]);
    assert_eq!(
        before.trim(),
        "refs/heads/main",
        "a refused name wrote a ref"
    );

    branch::create(root, "vibe/one", &base).unwrap();
    assert_eq!(
        branch::tip(root, "vibe/one").unwrap().as_deref(),
        Some(base.as_str())
    );
    // Create-only: the second create fails and the branch does not move.
    repo.write("a.txt", "b\n");
    repo.git(&["commit", "-qam", "two"]);
    let newer = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    let err = branch::create(root, "vibe/one", &newer).unwrap_err();
    assert!(
        matches!(err, vw_worktrees::WorktreeError::Taken(_)),
        "{err:?}"
    );
    assert_eq!(
        branch::tip(root, "vibe/one").unwrap().as_deref(),
        Some(base.as_str())
    );

    // Delete-if-at: a branch that moved stays.
    repo.git(&["branch", "-f", "vibe/one", &newer]);
    assert!(!branch::delete_if_at(root, "vibe/one", &base).unwrap());
    assert!(branch::exists(root, "vibe/one").unwrap());
    assert!(branch::delete_if_at(root, "vibe/one", &newer).unwrap());
    assert!(!branch::exists(root, "vibe/one").unwrap());
}

#[test]
fn inspection_lists_what_a_removal_loses() {
    let repo = TempRepo::with_files(&[
        ("a.txt", "a\n"),
        ("b.txt", "b\n"),
        (".gitignore", "node_modules/\n.env\n"),
    ]);
    let wt = add_worktree(&repo.root, &repo.base, "one", "vibe/one");
    let clean = inspect(&wt, Some("vibe/one"), &scan(&repo)).unwrap();
    assert!(
        clean.exists && clean.is_clean() && clean.blockers.is_empty(),
        "{clean:?}"
    );
    assert_eq!(clean.branch.as_deref(), Some("vibe/one"));
    assert_eq!(
        clean.unmerged_total, 0,
        "a new branch at main has no commit of its own"
    );

    std::fs::write(wt.join("a.txt"), "edited\n").unwrap();
    std::fs::write(wt.join("new.txt"), "new\n").unwrap();
    // A staged change whose working file was put back: only the index holds it.
    std::fs::write(wt.join("b.txt"), "staged\n").unwrap();
    run(&wt, &["add", "b.txt"]);
    std::fs::write(wt.join("b.txt"), "b\n").unwrap();
    std::fs::create_dir_all(wt.join("node_modules/pkg")).unwrap();
    std::fs::write(wt.join("node_modules/pkg/index.js"), "x\n").unwrap();
    std::fs::write(wt.join(".env"), "SECRET=1\n").unwrap();
    let dirty = inspect(&wt, Some("vibe/one"), &scan(&repo)).unwrap();
    assert_eq!(
        dirty.paths(),
        [".env", "a.txt", "b.txt", "new.txt"],
        "{dirty:?}"
    );
    assert_eq!(
        dirty.files[0].kind,
        LostKind::Ignored,
        "an ignored .env is work, not build output"
    );
    assert_eq!(
        dirty.build_output, 1,
        "node_modules/ is counted as build output"
    );

    // Commits on the branch that no other branch holds.
    run(
        &wt,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=T",
            "commit",
            "-qam",
            "c1",
        ],
    );
    std::fs::write(wt.join("new.txt"), "newer\n").unwrap();
    run(&wt, &["add", "new.txt"]);
    run(
        &wt,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=T",
            "commit",
            "-qm",
            "c2",
        ],
    );
    let ahead = inspect(&wt, Some("vibe/one"), &scan(&repo)).unwrap();
    assert_eq!(ahead.unmerged_total, 2, "{ahead:?}");
    assert_eq!(ahead.unmerged.len(), 2);
    assert!(
        ahead.unmerged.iter().any(|c| c.ends_with(" c2")),
        "{:?}",
        ahead.unmerged
    );
    assert_eq!(ahead.paths(), [".env"], "{ahead:?}");
    // Merged into main: nothing of its own any more.
    repo.git(&["merge", "-q", "--ff-only", "vibe/one"]);
    let merged = inspect(&wt, Some("vibe/one"), &scan(&repo)).unwrap();
    assert_eq!(merged.unmerged_total, 0, "{merged:?}");
    // The scratch folder is cleaned up.
    let left: Vec<_> = std::fs::read_dir(scan(&repo).join("sessions"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "{left:?}");

    // Gone.
    let gone = inspect(&repo.base.join("missing"), None, &scan(&repo)).unwrap();
    assert!(!gone.exists && gone.is_clean());
}

#[test]
fn the_main_checkout_is_untouched_by_every_operation() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    // A realistic user state: a staged edit, an unstaged edit, an untracked file and a stash entry.
    repo.write("a.txt", "staged\n");
    repo.git(&["add", "a.txt"]);
    repo.write("b.txt", "unstaged\n");
    repo.write("u.txt", "untracked\n");
    repo.git(&[
        "stash",
        "push",
        "-q",
        "--keep-index",
        "-m",
        "kept",
        "--",
        "b.txt",
    ]);
    repo.write("b.txt", "unstaged again\n");
    let before = checkout(&repo.root);

    let info = read(&repo.root).unwrap().unwrap();
    assert!(vw_worktrees::preflight(&repo.root).unwrap().is_none());
    let base = vw_worktrees::repo_info::resolve_base(&repo.root, "main").unwrap();
    let taken: Vec<String> = info.branches.iter().map(|b| b.name.clone()).collect();
    let pick =
        naming::suggest("check the invariants", |b, _| taken.iter().any(|t| t == b)).unwrap();
    branch::create(&repo.root, &pick.branch, &base).unwrap();
    let wt = repo.base.join("wt").join(&pick.folder);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    // What Vibe does with the branch we made (test setup: plain git).
    run(
        &repo.root,
        &["worktree", "add", "-q", wt.to_str().unwrap(), &pick.branch],
    );
    std::fs::write(wt.join("x.txt"), "agent\n").unwrap();
    let found = inspect(&wt, Some(&pick.branch), &scan(&repo)).unwrap();
    assert_eq!(found.paths(), ["x.txt"]);
    let listed = vw_worktrees::repo_info::worktrees(&repo.root).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].branch.as_deref(), Some(pick.branch.as_str()));

    let after = checkout(&repo.root);
    assert_eq!(before, after, "the main checkout changed");
    // The only new ref is the session branch.
    let refs = repo.git(&["for-each-ref", "--format=%(refname)"]);
    assert_eq!(
        refs.lines().collect::<Vec<_>>(),
        [
            "refs/heads/main",
            &format!("refs/heads/{}", pick.branch),
            "refs/stash"
        ]
    );
}
