//! When the core cannot record that it owns a session (its session-ownership folder is unusable), the
//! failure is the core's own (`internal`), never a Vibe server error. A session the failed request had just
//! created is deleted again; an existing session is left as it was.

mod common;

use std::path::{Path, PathBuf};

use common::core::{ScriptRig, script_core_with};
use common::fake::scratch;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreError, ErrorCode, Origin};

/// A core whose session bookkeeping lives in `<dir>/sessions`, with one stored session (`keep`) and a log of
/// where each request arrived. Returns the rig, the sessions folder and the channel log.
async fn rig(name: &str) -> (ScriptRig, PathBuf, PathBuf) {
    let dir = scratch(&format!("{name}-ownership"));
    let seed = dir.join("seed.json");
    let items = json!([
        {"id": "keep", "title": "Kept", "status": {"type": "idle"}, "createdAt": 3, "updatedAt": 8,
         "cwd": "/work/calc", "history": []},
    ]);
    std::fs::write(&seed, serde_json::to_vec(&items).unwrap()).unwrap();
    let seed = seed.to_string_lossy().into_owned();
    let channels = dir.join("channels.log");
    let channels_text = channels.to_string_lossy().into_owned();
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let sessions_for = sessions.clone();
    let rig = script_core_with(
        name,
        &[
            "--seed",
            seed.as_str(),
            "--log-channels",
            channels_text.as_str(),
        ],
        move |c| c.sessions_dir = Some(sessions_for),
    )
    .await;
    (rig, sessions, channels)
}

/// Replaces the session-ownership folder with a plain file, so no new ownership can be recorded.
fn block_ownership(sessions: &Path) {
    let leases = sessions.join("leases");
    if leases.is_dir() {
        std::fs::remove_dir_all(&leases).unwrap();
    }
    std::fs::write(&leases, b"plain file").unwrap();
}

async fn execute(rig: &ScriptRig, command: Value) -> Result<CommandResult, CoreError> {
    rig.core
        .execute(Origin::Ui, serde_json::from_value(command).unwrap())
        .await
}

fn session_id(result: &CommandResult) -> String {
    match result {
        CommandResult::Session { snapshot, .. } => snapshot.session_id.clone(),
        other => panic!("{other:?}"),
    }
}

async fn listed(rig: &ScriptRig) -> Vec<String> {
    let command = Command::ListSessions {
        limit: Some(50),
        cursor: None,
        include_archived: Some(true),
    };
    match rig.core.execute(Origin::Ui, command).await.unwrap() {
        CommandResult::Sessions { rows, .. } => {
            let mut ids: Vec<String> = rows
                .into_iter()
                .map(|r| r.session["id"].as_str().unwrap_or_default().to_owned())
                .collect();
            ids.sort();
            ids
        }
        other => panic!("{other:?}"),
    }
}

/// The session ids of the `session/delete` requests in a `--log-channels` file.
fn deletes(channels: &Path) -> Vec<String> {
    std::fs::read_to_string(channels)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(' ').skip(1);
            (parts.next()? == "session/delete").then(|| parts.next().map(str::to_owned))?
        })
        .collect()
}

/// The core's own failure for an ownership folder it cannot use.
fn assert_local_ownership_error(error: &CoreError) {
    assert_eq!(error.code, ErrorCode::Internal, "{error:?}");
    assert_eq!(error.server_code, None, "{error:?}");
    assert!(
        error
            .message
            .starts_with("Could not check session ownership"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn an_ordinary_start_is_undone_when_ownership_cannot_be_recorded() {
    let (r, sessions, channels) = rig("start").await;
    block_ownership(&sessions);
    let before = listed(&r).await;

    let error = execute(&r, json!({"type": "startSession"}))
        .await
        .expect_err("a session this app cannot own must not start");
    assert_local_ownership_error(&error);

    let deleted = deletes(&channels);
    assert_eq!(deleted.len(), 1, "{:?}", r.received());
    assert!(
        deleted[0].starts_with("fake-session-"),
        "the delete names the new session: {deleted:?}"
    );
    assert_eq!(
        listed(&r).await,
        before,
        "the started session is gone again"
    );
    r.core.shutdown().await;
}

#[tokio::test]
async fn an_attaching_fork_is_undone_when_ownership_cannot_be_recorded() {
    let (r, sessions, channels) = rig("fork").await;
    let source = session_id(
        &execute(&r, json!({"type": "startSession"}))
            .await
            .expect("the source starts while ownership can be recorded"),
    );
    // The source keeps the ownership it already holds; the fork cannot get its own.
    block_ownership(&sessions);
    let before = listed(&r).await;
    assert!(before.contains(&source), "{before:?}");

    let error = execute(
        &r,
        json!({"type": "forkSession", "sourceSessionId": source}),
    )
    .await
    .expect_err("a fork this app cannot own must not stay");
    assert_local_ownership_error(&error);

    let deleted = deletes(&channels);
    assert_eq!(deleted.len(), 1, "{:?}", r.received());
    assert!(
        deleted[0].starts_with("fake-fork-"),
        "the delete names the new fork, never its source: {deleted:?}"
    );
    assert_eq!(
        listed(&r).await,
        before,
        "the fork is gone again and the source is still there"
    );
    r.core.shutdown().await;
}

#[tokio::test]
async fn an_existing_session_is_kept_when_its_resume_cannot_record_ownership() {
    let (r, sessions, channels) = rig("resume").await;
    block_ownership(&sessions);
    let before = listed(&r).await;
    assert!(before.iter().any(|id| id == "keep"), "{before:?}");

    let error = execute(&r, json!({"type": "resumeSession", "sessionId": "keep"}))
        .await
        .expect_err("a session this app cannot own must not reopen");
    assert_local_ownership_error(&error);

    assert!(
        deletes(&channels).is_empty(),
        "nothing is deleted: {:?}",
        r.received()
    );
    assert!(
        !r.received().iter().any(|m| m == "session/resume"),
        "ownership is checked before Vibe reopens it: {:?}",
        r.received()
    );
    assert_eq!(listed(&r).await, before, "the stored session is kept");
    r.core.shutdown().await;
}
