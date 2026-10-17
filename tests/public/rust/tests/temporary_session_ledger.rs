//! Temporary session records remain durable and report failures.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::core::{ScriptRig, script_core_with};
use common::fake::scratch;
use common::logs;
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, CoreError, ErrorCode, Origin, SessionToolsResult};

/// Two stored sessions: one the user keeps, one a crashed run left behind as temporary.
fn seed_file(dir: &Path) -> String {
    let items = json!([
        {"id": "keep", "title": "Kept", "status": {"type": "idle"}, "createdAt": 3, "updatedAt": 8,
         "cwd": "/work/calc", "history": []},
        {"id": "left-temp", "title": "Left temporary", "status": {"type": "idle"}, "createdAt": 4, "updatedAt": 7,
         "cwd": "/work/calc", "history": []},
    ]);
    let path = dir.join("seed.json");
    std::fs::write(&path, serde_json::to_vec(&items).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

/// A core whose record lives in `sessions_dir` (which `prepare` may fill before the core starts).
async fn rig(
    name: &str,
    sessions_dir: impl FnOnce(&Path) -> PathBuf,
    prepare: impl FnOnce(&Path),
) -> (ScriptRig, PathBuf) {
    rig_with_flags(name, &[], sessions_dir, prepare).await
}

/// [`rig`] with extra flags for the fake Vibe.
async fn rig_with_flags(
    name: &str,
    flags: &[&str],
    sessions_dir: impl FnOnce(&Path) -> PathBuf,
    prepare: impl FnOnce(&Path),
) -> (ScriptRig, PathBuf) {
    let dir = scratch(&format!("{name}-tmp-ledger"));
    let seed = seed_file(&dir);
    let sessions_dir = sessions_dir(&dir);
    prepare(&dir);
    let dir_for = sessions_dir.clone();
    let mut args = vec!["--seed", seed.as_str()];
    args.extend_from_slice(flags);
    let rig = script_core_with(name, &args, move |c| {
        c.sessions_dir = Some(dir_for);
    })
    .await;
    (rig, sessions_dir)
}

async fn tools(rig: &ScriptRig, request: Value) -> Result<SessionToolsResult, CoreError> {
    let command: Command =
        serde_json::from_value(json!({"type": "sessionTools", "request": request})).unwrap();
    match rig.core.execute(Origin::Ui, command).await? {
        CommandResult::SessionTools { result } => Ok(result),
        other => panic!("{other:?}"),
    }
}

async fn start_temporary(rig: &ScriptRig) -> Result<String, CoreError> {
    match tools(rig, json!({"op": "startTemporary"})).await? {
        SessionToolsResult::Started { session_id } => Ok(session_id),
        other => panic!("{other:?}"),
    }
}

async fn temporary_ids(rig: &ScriptRig) -> Vec<String> {
    match tools(rig, json!({"op": "temporary"})).await.unwrap() {
        SessionToolsResult::Temporary { session_ids } => session_ids,
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

/// The ids in the record file, `None` when there is no readable list.
fn record(sessions_dir: &Path) -> Option<Vec<String>> {
    serde_json::from_slice(&std::fs::read(sessions_dir.join("temporary.json")).ok()?).ok()
}

/// The files the core moved aside: (path, bytes).
fn aside(sessions_dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("temporary.json.bad-")
        })
        .map(|e| (e.path(), std::fs::read(e.path()).unwrap_or_default()))
        .collect()
}

/// True when some log line is at error level and mentions `needle`.
fn logged_error(log: &logs::LogBuffer, needle: &str) -> bool {
    log.text()
        .lines()
        .any(|l| l.contains(" ERROR ") && l.contains(needle))
}

async fn wait_until_gone(rig: &ScriptRig, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let ids = listed(rig).await;
        if !ids.iter().any(|i| i == id) {
            return;
        }
        assert!(Instant::now() < deadline, "{id} still listed: {ids:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn normal_path_is_unchanged_and_durable_when_the_start_returns() {
    let (r, dir) = rig("normal", |d| d.join("sessions"), |_| {}).await;
    let id = start_temporary(&r).await.unwrap();
    // The answer is back, so the record must already name the session.
    assert_eq!(record(&dir), Some(vec![id.clone()]));
    assert_eq!(temporary_ids(&r).await, vec![id.clone()]);
    assert!(aside(&dir).is_empty());
    r.core.shutdown().await;
    assert!(
        r.received().iter().any(|m| m == "session/delete"),
        "{:?}",
        r.received()
    );
    assert_eq!(record(&dir), Some(Vec::new()));
}

/// The record cannot be written after the core started: the start fails and the new session is gone again.
async fn assert_failed_start_leaves_nothing(name: &str, flags: &[&str]) {
    let (log, _guard) = logs::capture();
    let (r, dir) = rig_with_flags(name, flags, |d| d.join("sessions"), |_| {}).await;
    let before = listed(&r).await;
    // Block the record itself, leaving the session-ownership directory usable.
    std::fs::create_dir_all(dir.join("temporary.json")).unwrap();

    let error = start_temporary(&r)
        .await
        .expect_err("the start must not succeed without a durable record");
    assert_eq!(error.code, ErrorCode::Internal, "{error:?}");
    assert!(error.message.contains("record"), "{}", error.message);

    assert!(
        temporary_ids(&r).await.is_empty(),
        "nothing is on the list that is not on disk"
    );
    assert!(
        r.received().iter().any(|m| m == "session/delete"),
        "the new session was deleted again: {:?}",
        r.received()
    );
    // The list merges what Vibe lists with what only the core's store knows, so a session the core kept after a
    // refused start would show up here.
    assert_eq!(
        listed(&r).await,
        before,
        "no stray session is left in the list"
    );
    assert!(logged_error(&log, "temporary"), "{}", log.text());
    r.core.shutdown().await;
}

#[tokio::test]
async fn start_fails_and_the_new_session_is_removed_when_the_record_cannot_be_written() {
    assert_failed_start_leaves_nothing("unwritable", &[]).await;
}

#[tokio::test]
async fn same_when_vibe_answers_not_found_for_the_never_run_session() {
    // The synthetic delete answers "not found" for an unstored session. The core still
    // has to finish its own bookkeeping.
    assert_failed_start_leaves_nothing("missing-after-start", &["--unstored-metadata-miss"]).await;
}

#[tokio::test]
async fn start_fails_when_the_record_is_unreadable_and_cannot_be_moved_aside() {
    let (log, _guard) = logs::capture();
    // `blocker` is a plain file, so nothing below it can be read, moved or created.
    let (r, _dir) = rig(
        "unreadable",
        |d| d.join("blocker").join("sessions"),
        |d| std::fs::write(d.join("blocker"), b"plain file").unwrap(),
    )
    .await;
    let before = listed(&r).await;
    let error = start_temporary(&r)
        .await
        .expect_err("no durable record is possible");
    assert_eq!(error.code, ErrorCode::Internal, "{error:?}");
    assert!(temporary_ids(&r).await.is_empty());
    assert_eq!(listed(&r).await, before, "no session was created");
    // Unix reports a path below a plain file as "not a directory", which freezes the record at load, before any
    // session exists. Windows reports "not found" there, so the failure only shows when the record is written.
    #[cfg(unix)]
    assert!(
        !r.received().iter().any(|m| m == "session/start"),
        "{:?}",
        r.received()
    );
    assert!(logged_error(&log, "temporary"), "{}", log.text());
    r.core.shutdown().await;
}

/// Runs a core on a record with `content`; returns (rig, folder, the record's original bytes, log).
async fn damaged(
    name: &str,
    content: &[u8],
) -> (
    ScriptRig,
    PathBuf,
    logs::LogBuffer,
    tracing::subscriber::DefaultGuard,
) {
    let (log, guard) = logs::capture();
    let bytes = content.to_vec();
    let (r, dir) = rig(
        name,
        |d| d.join("sessions"),
        |d| {
            std::fs::create_dir_all(d.join("sessions")).unwrap();
            std::fs::write(d.join("sessions").join("temporary.json"), &bytes).unwrap();
        },
    )
    .await;
    (r, dir, log, guard)
}

/// A damaged record: moved aside byte for byte, logged, and the whole ids in it are swept.
async fn assert_aside_and_swept(name: &str, content: &[u8], kept_ids_swept: &[&str]) {
    let (r, dir, log, _guard) = damaged(name, content).await;
    let moved = aside(&dir);
    assert_eq!(moved.len(), 1, "exactly one copy moved aside: {moved:?}");
    assert_eq!(
        moved[0].1, content,
        "the damaged record is preserved unchanged"
    );
    assert!(logged_error(&log, "temporary"), "{}", log.text());
    for id in kept_ids_swept {
        wait_until_gone(&r, id).await;
    }
    let ids = listed(&r).await;
    assert!(
        ids.iter().any(|i| i == "keep"),
        "a kept session is never deleted: {ids:?}"
    );
    // After the sweep the list is empty again and a readable record (if any) agrees.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !temporary_ids(&r).await.is_empty() {
        assert!(Instant::now() < deadline, "the list never emptied");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(record(&dir).is_none_or(|ids| ids.is_empty()));
    // The core keeps working: a new temporary chat can be recorded again.
    let id = start_temporary(&r).await.unwrap();
    assert_eq!(record(&dir), Some(vec![id]));
    r.core.shutdown().await;
}

#[tokio::test]
async fn truncated_record_keeps_its_whole_ids() {
    // `keep` is cut off inside its quotes: it is not a whole id and must not be taken for one.
    assert_aside_and_swept("truncated", br#"["left-temp", "keep"#, &["left-temp"]).await;
}

#[tokio::test]
async fn record_with_a_non_string_entry_keeps_its_string_ids() {
    assert_aside_and_swept("mixed-list", br#"["left-temp", 7, null]"#, &["left-temp"]).await;
}

#[tokio::test]
async fn record_that_is_no_list_keeps_nothing_and_is_still_preserved() {
    let (r, dir, log, _guard) = damaged("invalid-json", b"this is not json").await;
    let moved = aside(&dir);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0].1, b"this is not json");
    assert!(logged_error(&log, "temporary"), "{}", log.text());
    assert!(temporary_ids(&r).await.is_empty());
    // Nothing was guessed: both stored sessions are still there.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        listed(&r).await,
        vec!["keep".to_owned(), "left-temp".to_owned()]
    );
    r.core.shutdown().await;
}

#[tokio::test]
async fn unreadable_record_is_moved_aside_not_replaced() {
    let (log, _guard) = logs::capture();
    let (r, dir) = rig(
        "record-folder",
        |d| d.join("sessions"),
        |d| std::fs::create_dir_all(d.join("sessions").join("temporary.json")).unwrap(),
    )
    .await;
    // A folder where the file belongs cannot be read as a list; it is moved aside whole, never deleted.
    let moved = aside(&dir);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert!(moved[0].0.is_dir(), "{moved:?}");
    assert!(logged_error(&log, "temporary"), "{}", log.text());
    assert!(temporary_ids(&r).await.is_empty());
    // The record works again afterwards.
    let id = start_temporary(&r).await.unwrap();
    assert_eq!(record(&dir), Some(vec![id]));
    r.core.shutdown().await;
}

#[tokio::test]
async fn valid_record_is_not_moved_aside() {
    let (log, _guard) = logs::capture();
    let (r, dir) = rig(
        "valid-list",
        |d| d.join("sessions"),
        |d| {
            std::fs::create_dir_all(d.join("sessions")).unwrap();
            std::fs::write(
                d.join("sessions").join("temporary.json"),
                br#"["left-temp"]"#,
            )
            .unwrap();
        },
    )
    .await;
    wait_until_gone(&r, "left-temp").await;
    assert!(aside(&dir).is_empty());
    assert!(!logged_error(&log, "temporary"), "{}", log.text());
    r.core.shutdown().await;
}

#[tokio::test]
async fn quitting_with_an_untouched_temporary_chat_empties_the_record() {
    let (r, dir) = rig_with_flags(
        "untouched-chat",
        &["--unstored-metadata-miss"],
        |d| d.join("sessions"),
        |_| {},
    )
    .await;
    let id = start_temporary(&r).await.unwrap();
    assert_eq!(record(&dir), Some(vec![id]));
    r.core.shutdown().await;
    assert_eq!(
        record(&dir),
        Some(Vec::new()),
        "Vibe's not_found counts as deleted"
    );
}
