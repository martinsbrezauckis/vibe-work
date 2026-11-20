//! An app push sends only the confirmed branch ref, even when the person's own Git configuration says
//! `push.followTags = true` (an annotated tag a repository planted on the pushed commit would otherwise
//! leave with the branch and could start tag-based release automation; the card names only the branch).
//! The outer test re-runs this binary's ignored inner case with HOME pointing at a fixture home, so the
//! person's configuration is simulated without changing this process's environment.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

fn fixture_base() -> PathBuf {
    std::env::temp_dir().join(format!("vw-git-push-no-tags-{}", std::process::id()))
}

#[test]
fn app_push_never_sends_tags_the_card_did_not_name() {
    let base = fixture_base();
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".gitconfig"),
        "[push]\n\tfollowTags = true\n\trecurseSubmodules = on-demand\n",
    )
    .unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inner_push_with_person_follow_tags",
            "--ignored",
            "--nocapture",
        ])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("VW_GIT_PUSH_NO_TAGS_BASE", &base)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&base);
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("1 passed"),
        "inner case failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "run by app_push_never_sends_tags_the_card_did_not_name with a fixture HOME"]
fn inner_push_with_person_follow_tags() {
    let base = PathBuf::from(std::env::var_os("VW_GIT_PUSH_NO_TAGS_BASE").unwrap());
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "test@example.invalid"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "initial"]);
    git(
        &repo,
        &["tag", "-a", "release-v9", "-m", "planted by the repository"],
    );
    let remote = base.join("remote.git");
    git(&base, &["init", "-q", "--bare", remote.to_str().unwrap()]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    vw_changes::prepare_root(&base.join("changes")).unwrap();
    let state = vw_git::state::read_state(&repo, &repo.join(".git"), None).unwrap();
    let plan = vw_git::push::plan_local_branch(&repo, &state, None, None).unwrap();
    vw_git::push::push_local_branch(&repo, &plan).unwrap();
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]), plan.commit);
    assert_eq!(
        git(&remote, &["tag", "--list"]),
        "",
        "the push sent a tag the card did not name"
    );
}
