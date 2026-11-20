//! A chat's vault identity after its agent moves into a linked Git worktree (`Identity::moved_into`, used by
//! `permission_project_of` for secret runs, login fills and the secret list).
//!
//! PREDICT (written before the change): the session-open identity accepted only the exact folder it captured, so
//! `the_same_projects_linked_copy_keeps_the_projects_identity` and `going_back_to_the_project_folder_is_accepted`
//! fail with "this chat's folder no longer matches its captured project identity". The refusals below hold
//! before and after the change: an unrelated folder, another project's worktree, a `.git` link that only points one
//! way, and a copy of a project whose own identity never included its main checkout.

use std::path::Path;

use serde_json::json;

use crate::agent_tools::permission_project_of;

const MOVED: &str = "this chat's folder no longer matches its captured project identity";

fn git(folder: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
        ])
        .args(args)
        .current_dir(folder)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository at `main` with one commit and a linked worktree at `copy`.
fn repo_with_copy(main: &Path, copy: &Path) {
    std::fs::create_dir_all(main).unwrap();
    git(main, &["init", "-q"]);
    git(main, &["commit", "-q", "--allow-empty", "-m", "fixture"]);
    let branch = copy.file_name().unwrap().to_str().unwrap();
    git(
        main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            copy.to_str().unwrap(),
        ],
    );
}

async fn open(shared: &crate::core::Shared, id: &str, cwd: &Path) {
    let opening = crate::entry_projects::before_open(
        shared,
        "session/start",
        &json!({"agentConfig":{"cwd":cwd}}),
    )
    .await
    .unwrap();
    opening.finish(shared, &json!({"state":{"session":{"id":id}}}));
}

fn canonical(path: &Path) -> std::path::PathBuf {
    vw_platform::canonical_plain(path).unwrap()
}

#[tokio::test]
async fn the_same_projects_linked_copy_keeps_the_projects_identity() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let (main, copy) = (fixture.root.join("main"), fixture.root.join("copies/copy"));
    repo_with_copy(&main, &copy);
    let shared = &fixture.core.shared;
    open(shared, "chat", &main).await;
    let (main, copy) = (canonical(&main), canonical(&copy));
    let original = permission_project_of(shared, "chat", std::slice::from_ref(&main)).unwrap();
    let moved = permission_project_of(shared, "chat", std::slice::from_ref(&copy))
        .expect("the chat's agent entered its project's linked copy");
    assert_eq!(moved.allowed_project(), original.allowed_project());
    assert_eq!(moved.allowed_project(), main.to_string_lossy());
    fixture.finish().await;
}

#[tokio::test]
async fn going_back_to_the_project_folder_is_accepted() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let (main, copy) = (fixture.root.join("main"), fixture.root.join("copies/back"));
    repo_with_copy(&main, &copy);
    let shared = &fixture.core.shared;
    // The project's main folder is the person's: a chat opened in its copy inherits it.
    fixture.core.register_picked_folder(&main).unwrap();
    open(shared, "chat", &copy).await;
    let (main, copy) = (canonical(&main), canonical(&copy));
    let opened = permission_project_of(shared, "chat", std::slice::from_ref(&copy)).unwrap();
    assert_eq!(opened.allowed_project(), main.to_string_lossy());
    let back = permission_project_of(shared, "chat", std::slice::from_ref(&main))
        .expect("the agent left the copy for the project folder");
    assert_eq!(back.allowed_project(), main.to_string_lossy());
    fixture.finish().await;
}

#[tokio::test]
async fn any_other_folder_is_refused() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let root = &fixture.root;
    let (main, copy) = (root.join("main"), root.join("copies/copy"));
    repo_with_copy(&main, &copy);
    // Another project with its own linked copy.
    let (other, other_copy) = (root.join("other"), root.join("copies/other-copy"));
    repo_with_copy(&other, &other_copy);
    // A plain folder, and a folder whose `.git` names the project's worktree entry that does not point back to it.
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let forged = root.join("forged");
    std::fs::create_dir_all(&forged).unwrap();
    std::fs::write(
        forged.join(".git"),
        format!("gitdir: {}\n", main.join(".git/worktrees/copy").display()),
    )
    .unwrap();
    let shared = &fixture.core.shared;
    open(shared, "chat", &main).await;
    for folder in [&plain, &other, &other_copy, &forged] {
        let folder = canonical(folder);
        assert_eq!(
            permission_project_of(shared, "chat", std::slice::from_ref(&folder)).err(),
            Some(MOVED.to_owned()),
            "{}",
            folder.display()
        );
    }
    assert_eq!(
        permission_project_of(shared, "chat", &[]).err(),
        Some(MOVED.to_owned())
    );
    fixture.finish().await;
}

#[tokio::test]
async fn a_copy_whose_identity_never_held_its_main_moves_nowhere() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let (main, first) = (fixture.root.join("main"), fixture.root.join("copies/first"));
    repo_with_copy(&main, &first);
    let second = fixture.root.join("copies/second");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "second",
            second.to_str().unwrap(),
        ],
    );
    let shared = &fixture.core.shared;
    // The main folder is not the person's: the chat opened in the first copy has only that folder's identity.
    open(shared, "chat", &first).await;
    let (main, second) = (canonical(&main), canonical(&second));
    for folder in [&main, &second] {
        assert_eq!(
            permission_project_of(shared, "chat", std::slice::from_ref(folder)).err(),
            Some(MOVED.to_owned()),
            "{}",
            folder.display()
        );
    }
    fixture.finish().await;
}

#[tokio::test]
async fn a_second_chat_keeps_its_own_identity() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let (main, copy) = (fixture.root.join("main"), fixture.root.join("copies/copy"));
    repo_with_copy(&main, &copy);
    let (other, other_copy) = (fixture.root.join("other"), fixture.root.join("copies/o"));
    repo_with_copy(&other, &other_copy);
    let shared = &fixture.core.shared;
    open(shared, "first", &main).await;
    open(shared, "second", &other).await;
    let copy = canonical(&copy);
    assert!(permission_project_of(shared, "first", std::slice::from_ref(&copy)).is_ok());
    assert_eq!(
        permission_project_of(shared, "second", std::slice::from_ref(&copy)).err(),
        Some(MOVED.to_owned()),
        "another chat's project copy is not this chat's"
    );
    fixture.finish().await;
}

/// A Never the person set on the folder a chat opened in still refuses after the agent moves: the moved identity
/// reads that folder's record too (`policy` turns any Never among its records into Never).
#[tokio::test]
async fn the_folders_a_chat_had_stay_deny_candidates_after_a_move() {
    let fixture = crate::feature_test_core::Fixture::new().await;
    vw_changes::prepare_root(&fixture.root.join("changes")).unwrap();
    let (main, first) = (fixture.root.join("main"), fixture.root.join("copies/first"));
    repo_with_copy(&main, &first);
    let second = fixture.root.join("copies/second");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "second",
            second.to_str().unwrap(),
        ],
    );
    fixture.core.register_picked_folder(&main).unwrap();
    let shared = &fixture.core.shared;
    open(shared, "in-copy", &first).await;
    open(shared, "in-main", &main).await;
    let (main, first, second) = (canonical(&main), canonical(&first), canonical(&second));
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let to_main = permission_project_of(shared, "in-copy", std::slice::from_ref(&main)).unwrap();
    assert_eq!(to_main.denies, [text(&first)]);
    let to_second =
        permission_project_of(shared, "in-copy", std::slice::from_ref(&second)).unwrap();
    assert_eq!(to_second.denies, [text(&first)]);
    assert_eq!(to_second.main.as_deref(), Some(text(&main).as_str()));
    // Opened in the project folder: that folder stays a candidate (`main`) after entering a copy.
    let to_copy = permission_project_of(shared, "in-main", std::slice::from_ref(&first)).unwrap();
    assert!(to_copy.denies.is_empty(), "{:?}", to_copy.denies);
    assert_eq!(to_copy.main.as_deref(), Some(text(&main).as_str()));
    fixture.finish().await;
}
