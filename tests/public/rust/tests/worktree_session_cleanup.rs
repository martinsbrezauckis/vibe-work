//! Real Vibe with a scripted model verifies explicit removal and compatibility with legacy copy records.

// Unix only, like these tests before they became public: on Windows git reports a worktree as `C:/...` while the
// temporary folder resolves to `\\?\C:\...`, so the path comparisons and readiness waits do not match there.
// The skip is listed by name in `platform_skips.rs`.
#![cfg(unix)]

mod changes;
mod common;
mod worktrees;

use std::path::{Path, PathBuf};
use std::time::Duration;

use changes::TempRepo;
use common::core::{ScriptRig, real_core};
use serde_json::{Value, json};
use vw_core_api::{
    BranchPlan, ChangesResult, Command, CommandResult, CoreError, CoreEvent, ErrorCode, Origin,
    WorktreeInfo, WorktreeInspection, WorktreeState, WorktreesResult,
};
use vw_gate::Confirmation;
use worktrees::{checkout, run};

async fn rig(name: &str, data: &Path) -> ScriptRig {
    let data = data.to_path_buf();
    real_core(
        name,
        &[],
        None,
        common::real_vibe::edit_script(),
        move |config| {
            config.changes_dir = Some(data);
        },
    )
    .await
    .0
}

async fn exec_as(
    rig: &ScriptRig,
    origin: Origin,
    value: Value,
) -> Result<CommandResult, CoreError> {
    rig.core
        .execute(origin, serde_json::from_value::<Command>(value).unwrap())
        .await
}

async fn wt(rig: &ScriptRig, origin: Origin, request: Value) -> Result<WorktreesResult, CoreError> {
    match exec_as(
        rig,
        origin,
        json!({"type": "worktrees", "request": request}),
    )
    .await?
    {
        CommandResult::Worktrees { result } => Ok(result),
        other => panic!("{other:?}"),
    }
}

async fn start_copy(rig: &ScriptRig, folder: &Path, base: &str, branch: &str) -> String {
    rig.core.register_picked_folder(folder).unwrap();
    let result = wt(
        rig,
        Origin::Ui,
        json!({"op": "start", "agentConfig": {"cwd": folder.to_str().unwrap()}, "base": base, "branch": branch}),
    )
    .await
    .unwrap();
    let WorktreesResult::Started { snapshot, info, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(info.branch, branch);
    assert_eq!(info.base.as_deref(), Some(base));
    let sid = snapshot.session_id;
    exec_as(
        rig,
        Origin::Ui,
        json!({"type": "setAccessMode", "sessionId": sid, "agentName": "accept-edits"}),
    )
    .await
    .unwrap();
    sid
}

async fn info(rig: &ScriptRig, sid: &str) -> Option<WorktreeInfo> {
    match wt(rig, Origin::Ui, json!({"op": "info", "sessionId": sid}))
        .await
        .unwrap()
    {
        WorktreesResult::Info { info } => info,
        other => panic!("{other:?}"),
    }
}

/// Waits until the session's copy is ready and the session's folder moved into it.
async fn ready(rig: &ScriptRig, sid: &str) -> PathBuf {
    for _ in 0..100 {
        if let Some(i) = info(rig, sid).await
            && i.state == WorktreeState::Ready
            && let Some(path) = i.path
        {
            let cwd = match exec_as(
                rig,
                Origin::Ui,
                json!({"type": "readSession", "sessionId": sid}),
            )
            .await
            {
                Ok(CommandResult::Session { snapshot, .. }) => {
                    snapshot.state["session"]["cwd"].as_str().map(str::to_owned)
                }
                _ => None,
            };
            if cwd.as_deref() == Some(path.as_str()) {
                return PathBuf::from(path);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "the copy of {sid} never became ready: {:?}",
        info(rig, sid).await
    );
}

async fn turn_with_edits(rig: &ScriptRig, sid: &str, text: &str) {
    let mut events = rig.core.subscribe(Some(sid.to_owned()));
    exec_as(
        rig,
        Origin::Ui,
        json!({"type": "startTurn", "sessionId": sid, "message": [{"type": "text", "text": text}]}),
    )
    .await
    .unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        let mut changed = false;
        let mut idle = false;
        while let Some(event) = events.next().await {
            changed |= matches!(event, CoreEvent::ChangesUpdated { .. });
            idle |= common::channel_rig::went_idle(&event);
            if changed && idle {
                return;
            }
        }
        panic!("session event stream ended before changes and idle");
    })
    .await;
    assert!(
        waited.is_ok(),
        "no changesUpdated and idle events within 60 s"
    );
}

/// One plain turn (no edits), waiting until the session is idle again.
async fn plain_turn(rig: &ScriptRig, sid: &str) {
    let mut events = rig.core.subscribe(Some(sid.to_owned()));
    exec_as(
        rig,
        Origin::Ui,
        json!({"type": "startTurn", "sessionId": sid, "message": [{"type": "text", "text": "hello"}]}),
    )
    .await
    .unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.next().await {
            if common::channel_rig::went_idle(&event) {
                return;
            }
        }
        panic!("session event stream ended before idle");
    })
    .await;
    assert!(waited.is_ok(), "no idle event within 60 s");
}

async fn changed(rig: &ScriptRig, sid: &str) -> Vec<String> {
    match exec_as(
        rig,
        Origin::Ui,
        json!({"type": "changes", "request": {"op": "read", "sessionId": sid}}),
    )
    .await
    {
        Ok(CommandResult::Changes {
            result: ChangesResult::View { view },
        }) => view.files.into_iter().map(|f| f.path).collect(),
        other => panic!("{other:?}"),
    }
}

async fn inspect(rig: &ScriptRig, sid: &str) -> WorktreeInspection {
    match wt(rig, Origin::Ui, json!({"op": "inspect", "sessionId": sid}))
        .await
        .unwrap()
    {
        WorktreesResult::Inspection { inspection } => inspection,
        other => panic!("{other:?}"),
    }
}

/// Every removal call the fake received: each must state the three parameters.
fn assert_explicit_removal_parameters(rig: &ScriptRig) -> usize {
    let log = std::fs::read_to_string(rig.dir.join("params.jsonl")).unwrap_or_default();
    let calls: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|call| call["method"] == "workspace/git/worktrees/remove")
        .map(|call| call["params"].clone())
        .collect();
    for call in &calls {
        let inspect = call["inspect"].as_bool();
        if inspect == Some(true) {
            continue;
        }
        assert!(
            call["force"].is_boolean()
                && call["deleteBranch"].is_boolean()
                && inspect == Some(false),
            "a removal did not carry explicit inspection and choice parameters: {call}"
        );
    }
    calls.len()
}

fn branches(repo: &TempRepo) -> Vec<String> {
    repo.git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn options_suggestions_and_refusals() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    repo.git(&["branch", "feature"]);
    let rig = rig("worktree-options", &repo.data).await;
    rig.core.register_picked_folder(&repo.root).unwrap();
    let cwd = repo.root.to_str().unwrap();
    let WorktreesResult::Options { options } =
        wt(&rig, Origin::Ui, json!({"op": "options", "cwd": cwd}))
            .await
            .unwrap()
    else {
        panic!()
    };
    assert!(options.is_git && options.offered, "{options:?}");
    let names: Vec<&str> = options.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["main", "feature"]);
    assert_eq!(options.suggestion.as_deref(), Some("vibe/session"));
    let WorktreesResult::Name { branch, folder } = wt(
        &rig,
        Origin::Ui,
        json!({"op": "suggestName", "cwd": cwd, "text": "Fix the login page"}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(
        (branch.as_str(), folder.as_str()),
        ("vibe/fix-the-login-page", "fix-the-login-page")
    );
    let plain = repo.base.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    rig.core.register_picked_folder(&plain).unwrap();
    let WorktreesResult::Options { options } = wt(
        &rig,
        Origin::Ui,
        json!({"op": "options", "cwd": plain.to_str().unwrap()}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert!(!options.is_git && !options.offered);

    // A client's own `agentConfig.worktree` never reaches Vibe.
    let before = rig
        .received()
        .iter()
        .filter(|m| *m == "session/start")
        .count();
    let raw = exec_as(
        &rig,
        Origin::Ui,
        json!({"type": "startSession", "agentConfig": {"cwd": cwd, "worktree": {"kind": "auto"}}}),
    )
    .await;
    assert_eq!(raw.unwrap_err().code, ErrorCode::InvalidParams);
    let raw = exec_as(&rig, Origin::Ui, json!({"type": "call", "method": "session/start",
        "params": {"agentConfig": {"cwd": cwd, "worktree": {"kind": "create", "branch": "x", "name": "x"}}}})).await;
    assert_eq!(raw.unwrap_err().code, ErrorCode::InvalidParams);
    assert_eq!(
        rig.received()
            .iter()
            .filter(|m| *m == "session/start")
            .count(),
        before
    );
    assert!(
        !rig.received()
            .iter()
            .any(|m| m.starts_with("workspace/git"))
    );

    // A repository whose own config names a filter program: not offered, nothing created.
    repo.git(&["config", "filter.evil.smudge", "touch /tmp/never"]);
    let refused = wt(
        &rig,
        Origin::Ui,
        json!({"op": "start", "agentConfig": {"cwd": cwd}, "base": "main", "branch": "vibe/x"}),
    )
    .await;
    let error = refused.unwrap_err();
    assert_eq!(error.code, ErrorCode::Denied);
    assert!(
        error.message.contains("filter.evil.smudge"),
        "{}",
        error.message
    );
    assert_eq!(branches(&repo), ["feature", "main"]);
    assert_eq!(
        rig.received()
            .iter()
            .filter(|m| *m == "session/start")
            .count(),
        before
    );
}

#[tokio::test]
async fn two_sessions_get_two_isolated_copies() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    repo.git(&["branch", "feature"]);
    repo.write("a.txt", "user's own uncommitted edit\n");
    repo.write("untracked.txt", "user\n");
    let before = checkout(&repo.root);
    let rig = rig("worktree-two", &repo.data).await;
    let a = start_copy(&rig, &repo.root, "feature", "vibe/a").await;
    let b = start_copy(&rig, &repo.root, "main", "vibe/b").await;
    let path_a = ready(&rig, &a).await;
    let path_b = ready(&rig, &b).await;
    assert_ne!(path_a, path_b);
    assert!(
        path_a.starts_with(rig.dir.join("vibe-home/worktrees")),
        "{path_a:?}"
    );
    assert_eq!(
        run(&repo.root, &["rev-parse", "vibe/a"]),
        run(&repo.root, &["rev-parse", "feature"]),
        "vibe/a starts at the chosen base"
    );
    turn_with_edits(&rig, &a, "edit:from-a.txt").await;
    turn_with_edits(&rig, &b, "edit:from-b.txt").await;
    assert!(path_a.join("from-a.txt").exists() && !path_a.join("from-b.txt").exists());
    assert!(path_b.join("from-b.txt").exists() && !path_b.join("from-a.txt").exists());
    assert!(!repo.path("from-a.txt").exists() && !repo.path("from-b.txt").exists());
    assert_eq!(changed(&rig, &a).await, ["from-a.txt"]);
    assert_eq!(changed(&rig, &b).await, ["from-b.txt"]);
    assert_eq!(checkout(&repo.root), before, "the main checkout changed");
    assert_explicit_removal_parameters(&rig);
}

#[tokio::test]
async fn removal_needs_the_exact_list_and_the_dialog() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let before = checkout(&repo.root);
    let rig = rig("worktree-remove", &repo.data).await;
    let sid = start_copy(&rig, &repo.root, "main", "vibe/dirty").await;
    let path = ready(&rig, &sid).await;
    turn_with_edits(&rig, &sid, "edit:new.txt").await;

    let seen = inspect(&rig, &sid).await;
    assert!(seen.blocked.is_none(), "{seen:?}");
    let files: Vec<&str> = seen.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(files, ["new.txt"]);
    assert_eq!(seen.branch_plan, BranchPlan::Deleted);
    assert!(seen.path.is_some());
    let WorktreesResult::Inspection { inspection: same } = wt(
        &rig,
        Origin::DebugApi,
        json!({"op": "inspect", "sessionId": sid}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert_eq!(same, seen, "the Debug API sees what the window sees");
    let print = seen.fingerprint.clone();

    let stale = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": sid, "fingerprint": "0000000000000000"}),
    )
    .await;
    assert_eq!(stale.unwrap_err().code, ErrorCode::Stale);
    assert!(
        path.join("new.txt").exists(),
        "a stale request removed the copy"
    );

    let shown = rig.confirmer.shown().len();
    rig.confirmer.then(Confirmation::Declined);
    let declined = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": sid, "fingerprint": print}),
    )
    .await;
    assert_eq!(declined.unwrap_err().code, ErrorCode::ConfirmationDeclined);
    assert!(
        path.join("new.txt").exists(),
        "a declined dialog removed the copy"
    );
    let dialog = rig.confirmer.shown()[shown].clone();
    assert_eq!(dialog.method, "workspace/git/worktrees/remove");
    assert_eq!(dialog.requester, vw_gate::Requester::Window);
    assert!(dialog.summary.contains("\"new.txt\""), "{}", dialog.summary);
    assert!(
        dialog.summary.contains("branch is deleted"),
        "{}",
        dialog.summary
    );
    let native = vw_host::dialog_text(&dialog).unwrap();
    assert!(native.contains("new.txt") && native.contains("branch is deleted"));
    assert!(
        !native.contains("workspace/git/worktrees/remove") && !native.contains("\"deleteBranch\"")
    );

    let removed = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": sid, "fingerprint": print}),
    )
    .await
    .unwrap();
    let WorktreesResult::Removed { outcome } = removed else {
        panic!("{removed:?}")
    };
    assert!(outcome.removed && outcome.branch_deleted, "{outcome:?}");
    assert!(!path.exists());
    assert_eq!(branches(&repo), ["main"]);
    assert_eq!(
        info(&rig, &sid).await.unwrap().state,
        WorktreeState::Removed
    );
    assert_eq!(checkout(&repo.root), before, "the main checkout changed");
    assert!(
        assert_explicit_removal_parameters(&rig) >= 4,
        "real Vibe saw the probes and the removal"
    );
}

#[tokio::test]
async fn a_branch_with_commits_is_kept_and_a_clean_copy_goes() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let rig = rig("worktree-commits", &repo.data).await;
    let sid = start_copy(&rig, &repo.root, "main", "vibe/kept").await;
    let path = ready(&rig, &sid).await;
    turn_with_edits(&rig, &sid, "edit:work.txt").await;
    std::fs::write(path.join("work.txt"), "committed\n").unwrap();
    run(&path, &["add", "work.txt"]);
    run(
        &path,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=T",
            "commit",
            "-qm",
            "work",
        ],
    );
    let seen = inspect(&rig, &sid).await;
    assert_eq!(seen.unmerged_total, 1);
    assert!(seen.unmerged[0].ends_with(" work"), "{seen:?}");
    assert!(seen.files.is_empty(), "{seen:?}");
    assert!(
        matches!(seen.branch_plan, BranchPlan::Kept { .. }),
        "{seen:?}"
    );
    let shown = rig.confirmer.shown().len();
    let WorktreesResult::Removed { outcome } = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": sid, "fingerprint": seen.fingerprint}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert!(outcome.removed && !outcome.branch_deleted, "{outcome:?}");
    assert!(
        rig.confirmer.shown()[shown]
            .summary
            .contains("no uncommitted changes")
    );
    assert!(!path.exists());
    assert!(
        branches(&repo).contains(&"vibe/kept".to_owned()),
        "the branch with a commit was deleted"
    );
    assert_explicit_removal_parameters(&rig);
}

/// A 2.26.1 claim has no session_dirs; Vibe 3 must not reap a legacy kept or undecided copy.
#[tokio::test]
async fn legacy_copies_stay_listed_and_removable_after_restart() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let original = rig("worktree-legacy", &repo.data).await;
    let home = original.vibe_home();
    let records = vw_worktrees::Records::open(&repo.data.join("worktrees/records")).unwrap();
    let mut copies = Vec::new();
    for (name, decision) in [
        ("old-kept", Some(vw_worktrees::Decision::Kept)),
        ("old-undecided", None),
    ] {
        let id = start_copy(&original, &repo.root, "main", &format!("vibe/{name}")).await;
        let path = ready(&original, &id).await;
        // A copy whose chat never ran is reaped by Vibe 3.0.2 when it shuts down (its session is not stored, so
        // the cleanup it schedules on close finishes; 3.0.0 exited before it did). A legacy copy is emulated on
        // a chat that ran, so Vibe still holds its session at shutdown.
        plain_turn(&original, &id).await;
        records
            .update(&id, |r| {
                r.session_deleted = true;
                r.decision = decision;
            })
            .unwrap();
        copies.push((id, path));
    }
    original.core.shutdown().await;
    for (_, path) in &copies {
        let claim = home
            .join("worktrees/.claims")
            .join(path.parent().unwrap().file_name().unwrap())
            .join(path.file_name().unwrap())
            .join("record.json");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&claim).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("session_dirs");
        value
            .as_object_mut()
            .unwrap()
            .remove("moved_checkout_admin");
        value["reap_requested"] = json!(false);
        std::fs::write(claim, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let data = repo.data.clone();
    let (reopened, _model) = real_core(
        "worktree-legacy-restart",
        &[],
        Some(&home),
        common::real_vibe::edit_script(),
        move |config| config.changes_dir = Some(data),
    )
    .await;
    let WorktreesResult::Orphans { orphans } =
        wt(&reopened, Origin::DebugApi, json!({"op":"orphans"}))
            .await
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(orphans.len(), 2, "{orphans:?}");
    for (id, path) in copies {
        assert!(path.join("a.txt").is_file());
        assert!(
            orphans
                .iter()
                .any(|o| o.session_id == id && o.path.as_deref() == path.to_str())
        );
        let seen = inspect(&reopened, &id).await;
        assert!(seen.blocked.is_none(), "{seen:?}");
        let WorktreesResult::Removed { outcome } = wt(
            &reopened,
            Origin::DebugApi,
            json!({"op":"remove", "sessionId":id, "fingerprint":seen.fingerprint}),
        )
        .await
        .unwrap() else {
            panic!()
        };
        assert!(outcome.removed && !path.exists(), "{outcome:?}");
    }
    reopened.core.shutdown().await;
}

/// path is ever taken from a client or inferred from the session's folder); a copy whose branch now lives in a
/// worktree outside Vibe's worktree folder is blocked and nothing is removed.
#[tokio::test]
async fn only_recorded_copies_inside_vibes_folder_are_acted_on() {
    let repo = TempRepo::with_files(&[("a.txt", "a\n")]);
    let rig = rig("worktree-authz", &repo.data).await;
    rig.core.register_picked_folder(&repo.root).unwrap();
    let plain = match exec_as(
        &rig,
        Origin::Ui,
        json!({"type": "startSession", "agentConfig": {"cwd": repo.root.to_str().unwrap()}}),
    )
    .await
    .unwrap()
    {
        CommandResult::Session { snapshot, .. } => snapshot.session_id,
        other => panic!("{other:?}"),
    };
    let none = wt(
        &rig,
        Origin::Ui,
        json!({"op": "inspect", "sessionId": plain}),
    )
    .await;
    assert_eq!(none.unwrap_err().code, ErrorCode::NotFound);
    let none = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": plain, "fingerprint": "x"}),
    )
    .await;
    assert_eq!(none.unwrap_err().code, ErrorCode::NotFound);

    let sid = start_copy(&rig, &repo.root, "main", "vibe/moved").await;
    let path = ready(&rig, &sid).await;
    // The branch's worktree moves outside Vibe's folder (plain git, as a user or agent could).
    run(
        &repo.root,
        &["worktree", "remove", "--force", path.to_str().unwrap()],
    );
    let outside = repo.base.join("outside");
    run(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            outside.to_str().unwrap(),
            "vibe/moved",
        ],
    );
    let seen = inspect(&rig, &sid).await;
    assert!(
        seen.blocked
            .as_deref()
            .unwrap_or("")
            .contains("not inside Vibe's worktree folder"),
        "{seen:?}"
    );
    let refused = wt(
        &rig,
        Origin::Ui,
        json!({"op": "remove", "sessionId": sid, "fingerprint": seen.fingerprint}),
    )
    .await;
    assert_eq!(refused.unwrap_err().code, ErrorCode::Denied);
    assert!(
        outside.join("a.txt").exists(),
        "the outside worktree was touched"
    );
}
