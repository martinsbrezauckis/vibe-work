//! Synthetic file edits verify stale writes, session handles, and path safety.

mod changes;
mod common;

use std::path::Path;

use changes::TempRepo;
use common::core::{ScriptRig, script_core_with};
use common::fake::scratch;
use serde_json::{Value, json};
use vw_core_api::{
    ChangesResult, Command, CommandResult, CoreError, ErrorCode, FilesResult, Origin,
};

async fn files(rig: &ScriptRig, request: Value) -> Result<FilesResult, CoreError> {
    let command: Command =
        serde_json::from_value(json!({"type": "files", "request": request})).unwrap();
    match rig.core.execute(Origin::Ui, command).await? {
        CommandResult::Files { result } => Ok(result),
        other => panic!("{other:?}"),
    }
}

async fn start(rig: &ScriptRig, folder: &Path) -> String {
    let command: Command =
        serde_json::from_value(json!({"type": "call", "method": "session/start",
        "params": {"agentConfig": {"cwd": folder.to_str().unwrap()}}}))
        .unwrap();
    match rig.core.execute(Origin::Ui, command).await.unwrap() {
        CommandResult::Reply { result, .. } => result["state"]["session"]["id"]
            .as_str()
            .unwrap()
            .to_owned(),
        other => panic!("{other:?}"),
    }
}

/// The handle and the fingerprint `read` gives for `path`.
async fn open(rig: &ScriptRig, session: &str, path: &str) -> (String, String) {
    let FilesResult::Resolved { refs } = files(
        rig,
        json!({"op": "resolve", "sessionId": session, "paths": [path]}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    let file = refs[0]
        .file
        .clone()
        .unwrap_or_else(|| panic!("{path} not resolved: {refs:?}"));
    let FilesResult::Content { content } = files(
        rig,
        json!({"op": "read", "sessionId": session, "file": file}),
    )
    .await
    .unwrap() else {
        panic!()
    };
    (file, content.fingerprint)
}

async fn write(
    rig: &ScriptRig,
    session: &str,
    file: &str,
    text: &str,
    print: &str,
) -> Result<FilesResult, CoreError> {
    files(rig, json!({"op": "write", "sessionId": session, "file": file, "text": text, "fingerprint": print})).await
}

/// A File tab save moves the user's line into the persisted Changes baseline. The test goes
/// through Core::files, then Core::changes; it fails if files_write omits record_user_save.
#[tokio::test]
async fn editor_save_survives_changes_file_revert() {
    let repo =
        TempRepo::with_files(&[("notes.txt", "agent line\nquiet one\nquiet two\nuser line\n")]);
    let data = repo.data.clone();
    let rig = script_core_with("editor-save-owner", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    let session = start(&rig, &repo.root).await;
    let baseline = repo.baseline(&session);
    drop(baseline);
    repo.write("notes.txt", "agent edit\nquiet one\nquiet two\nuser line\n");

    let (file, fingerprint) = open(&rig, &session, "notes.txt").await;
    write(
        &rig,
        &session,
        &file,
        "agent edit\nquiet one\nquiet two\nuser edit\n",
        &fingerprint,
    )
    .await
    .unwrap();

    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "read", "sessionId": session
    }}))
    .unwrap();
    let CommandResult::Changes {
        result: ChangesResult::View { view },
    } = rig.core.execute(Origin::Ui, command).await.unwrap()
    else {
        panic!("expected Changes view")
    };
    let agent_change = view
        .files
        .iter()
        .find(|change| change.path == "notes.txt")
        .unwrap();
    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "revert", "sessionId": session,
        "target": {"kind": "file", "path": "notes.txt", "blob": agent_change.blob}
    }}))
    .unwrap();
    rig.core.execute(Origin::Ui, command).await.unwrap();
    assert_eq!(
        repo.read("notes.txt"),
        "agent line\nquiet one\nquiet two\nuser edit\n"
    );
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_editor_save_rolls_back_changes_baseline() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = TempRepo::with_files(&[("notes.txt", "agent line\nuser line\n")]);
    let data = repo.data.clone();
    let rig = script_core_with("editor-save-rollback", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    let session = start(&rig, &repo.root).await;
    drop(repo.baseline(&session));
    repo.write("notes.txt", "agent edit\nuser line\n");
    let (file, fingerprint) = open(&rig, &session, "notes.txt").await;
    let original_mode = std::fs::metadata(&repo.root).unwrap().permissions().mode();
    std::fs::set_permissions(&repo.root, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = write(
        &rig,
        &session,
        &file,
        "agent edit\nuser edit\n",
        &fingerprint,
    )
    .await;
    std::fs::set_permissions(&repo.root, std::fs::Permissions::from_mode(original_mode)).unwrap();
    assert_eq!(result.unwrap_err().code, ErrorCode::Internal);
    assert_eq!(repo.read("notes.txt"), "agent edit\nuser line\n");
    let persisted = vw_changes::refresh(&repo.reload(&session)).unwrap();
    let persisted_file = persisted.file("notes.txt").unwrap();
    assert_eq!((persisted_file.added, persisted_file.removed), (1, 1));

    // A failed replacement must leave the baseline at the original user line. Revert may now
    // safely restore the agent line, without installing the save that did not reach disk.
    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "read", "sessionId": session
    }}))
    .unwrap();
    let CommandResult::Changes {
        result: ChangesResult::View { view },
    } = rig.core.execute(Origin::Ui, command).await.unwrap()
    else {
        panic!("expected Changes view")
    };
    let agent_change = view
        .files
        .iter()
        .find(|change| change.path == "notes.txt")
        .unwrap();
    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "revert", "sessionId": session,
        "target": {"kind": "file", "path": "notes.txt", "blob": agent_change.blob}
    }}))
    .unwrap();
    rig.core.execute(Origin::Ui, command).await.unwrap();
    assert_eq!(repo.read("notes.txt"), "agent line\nuser line\n");
    rig.core.shutdown().await;
}

/// A baseline persistence error after updating its index must not leave a live review that can
/// later revert from an attempted save whose bytes never reached disk.
#[tokio::test]
async fn failed_attribution_disables_unsafe_revert() {
    let repo = TempRepo::with_files(&[("notes.txt", "agent line\nuser line\n")]);
    let data = repo.data.clone();
    let rig = script_core_with("editor-attribution-failure", &[], move |config| {
        config.changes_dir = Some(data);
    })
    .await;
    let session = start(&rig, &repo.root).await;
    drop(repo.baseline(&session));
    repo.write("notes.txt", "agent edit\nuser line\n");
    let (file, fingerprint) = open(&rig, &session, "notes.txt").await;

    // `commit_index` reaches persist after changing base.idx; make that final write fail.
    let large = repo.data.join("sessions").join(&session).join("large.json");
    let original_large = std::fs::read(&large).unwrap();
    std::fs::remove_file(&large).unwrap();
    std::fs::create_dir(&large).unwrap();
    let result = write(
        &rig,
        &session,
        &file,
        "agent edit\nuser edit\n",
        &fingerprint,
    )
    .await;
    std::fs::remove_dir(&large).unwrap();
    std::fs::write(&large, original_large).unwrap();

    assert_eq!(result.unwrap_err().code, ErrorCode::Internal);
    assert_eq!(repo.read("notes.txt"), "agent edit\nuser line\n");
    let persisted = vw_changes::refresh(&repo.reload(&session)).unwrap();
    let persisted_file = persisted.file("notes.txt").unwrap();
    assert_eq!((persisted_file.added, persisted_file.removed), (1, 1));
    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "read", "sessionId": session
    }}))
    .unwrap();
    let CommandResult::Changes {
        result: ChangesResult::View { view },
    } = rig.core.execute(Origin::Ui, command).await.unwrap()
    else {
        panic!("expected Changes view")
    };
    assert!(
        view.baseline_lost,
        "failed attribution left a writable review"
    );
    let command: Command = serde_json::from_value(json!({"type": "changes", "request": {
        "op": "revert", "sessionId": session,
        "target": {"kind": "file", "path": "notes.txt", "blob": null}
    }}))
    .unwrap();
    assert_eq!(
        rig.core
            .execute(Origin::Ui, command)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Denied
    );
    assert_eq!(repo.read("notes.txt"), "agent edit\nuser line\n");
    rig.core.shutdown().await;
}

#[tokio::test]
async fn quick_edits() {
    let dir = scratch("file-write");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("notes.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(work.join("dos.txt"), "a\r\nb\r\n").unwrap();
    std::fs::write(work.join("bom.txt"), b"\xEF\xBB\xBFhello\n").unwrap();
    std::fs::write(work.join("blob.bin"), b"ab\0cd").unwrap();
    std::fs::write(work.join("big.txt"), "x".repeat(300 * 1024)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            work.join("notes.txt"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
    }
    let rig = script_core_with("file-write", &[], |_| {}).await;
    let session = start(&rig, &work).await;

    let (file, print) = open(&rig, &session, "notes.txt").await;
    let saved = write(&rig, &session, &file, "one\nTWO\nthree\n", &print)
        .await
        .unwrap();
    let FilesResult::Written { fingerprint, size } = saved else {
        panic!("{saved:?}")
    };
    assert_eq!(
        std::fs::read_to_string(work.join("notes.txt")).unwrap(),
        "one\nTWO\nthree\n"
    );
    assert_eq!(size, 14);
    let FilesResult::Stat {
        fingerprint: now, ..
    } = files(
        &rig,
        json!({"op": "stat", "sessionId": session, "file": file}),
    )
    .await
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(now.as_deref(), Some(fingerprint.as_str()));
    let leftovers: Vec<_> = std::fs::read_dir(&work)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".vw-edit-"))
        .collect();
    assert!(leftovers.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(work.join("notes.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o640, "permissions kept");
    }

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(work.join("notes.txt"), "the agent's version\n").unwrap();
    let stale = write(&rig, &session, &file, "mine\n", &fingerprint)
        .await
        .unwrap_err();
    assert_eq!(stale.code, ErrorCode::Stale, "{stale:?}");
    assert_eq!(
        std::fs::read_to_string(work.join("notes.txt")).unwrap(),
        "the agent's version\n"
    );

    let (dos, print) = open(&rig, &session, "dos.txt").await;
    write(&rig, &session, &dos, "a\nB\n", &print).await.unwrap();
    assert_eq!(std::fs::read(work.join("dos.txt")).unwrap(), b"a\r\nB\r\n");
    let (bom, print) = open(&rig, &session, "bom.txt").await;
    write(&rig, &session, &bom, "hello world\n", &print)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(work.join("bom.txt")).unwrap(),
        b"\xEF\xBB\xBFhello world\n"
    );

    let (blob, print) = open(&rig, &session, "blob.bin").await;
    let refused = write(&rig, &session, &blob, "text", &print)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidParams);
    assert_eq!(std::fs::read(work.join("blob.bin")).unwrap(), b"ab\0cd");
    let (big, print) = open(&rig, &session, "big.txt").await;
    let refused = write(&rig, &session, &big, "small", &print)
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidParams);
    let other = start(&rig, &work).await;
    let (notes, print) = open(&rig, &session, "notes.txt").await;
    let foreign = write(&rig, &other, &notes, "x", &print).await.unwrap_err();
    assert_eq!(foreign.code, ErrorCode::NotFound);
    rig.core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn links_out_of_the_folder_are_never_written_through() {
    use std::os::unix::fs::symlink;
    let dir = scratch("file-write-links");
    let work = dir.join("work");
    let outside = dir.join("outside");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("victim.txt");
    std::fs::write(&victim, "outside, untouched\n").unwrap();
    std::fs::write(work.join("notes.txt"), "inside\n").unwrap();
    std::fs::write(work.join("sub/note.txt"), "in sub\n").unwrap();
    symlink(&victim, work.join("link-out.txt")).unwrap();
    symlink(&outside, work.join("linkdir")).unwrap();
    let rig = script_core_with("file-write-links", &[], |_| {}).await;
    let session = start(&rig, &work).await;
    let resolves = |path: &'static str| {
        let rig = &rig;
        let session = session.clone();
        async move {
            let FilesResult::Resolved { refs } = files(
                rig,
                json!({"op": "resolve", "sessionId": session, "paths": [path]}),
            )
            .await
            .unwrap() else {
                panic!()
            };
            refs[0].ok
        }
    };
    assert!(
        !resolves("link-out.txt").await,
        "a file link out of the folder never resolves"
    );
    assert!(
        !resolves("linkdir/victim.txt").await,
        "a folder link out of the folder never resolves"
    );
    symlink(
        &victim,
        work.join(format!("notes.txt.vw-edit-{}", std::process::id())),
    )
    .unwrap();
    let (notes, print) = open(&rig, &session, "notes.txt").await;
    write(&rig, &session, &notes, "edited inside\n", &print)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "outside, untouched\n",
        "never written through a planted link"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("notes.txt")).unwrap(),
        "edited inside\n"
    );
    let (note, print) = open(&rig, &session, "sub/note.txt").await;
    std::fs::rename(work.join("sub"), dir.join("sub-moved")).unwrap();
    std::fs::write(outside.join("note.txt"), "in sub\n").unwrap();
    symlink(&outside, work.join("sub")).unwrap();
    let refused = write(&rig, &session, &note, "through the swap\n", &print).await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(
        std::fs::read_to_string(outside.join("note.txt")).unwrap(),
        "in sub\n",
        "nothing written outside"
    );
    rig.core.shutdown().await;
}

/// agent rewrites the file with text of the same length and restores its old modification time (`touch -r`
/// semantics); the user's draft, started from the old text, must be refused as stale and the agent's text kept.
/// the content hash in the fingerprint. A `stat` of the rewritten file differs from the fingerprint the draft read, so
/// the tab's poll notices the change too.
#[tokio::test]
async fn a_same_size_rewrite_with_the_old_time_is_still_stale() {
    let dir = scratch("file-write-same-size");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let path = work.join("plan.txt");
    std::fs::write(&path, "alpha beta\n").unwrap();
    let rig = script_core_with("file-write-same-size", &[], |_| {}).await;
    let session = start(&rig, &work).await;
    let (file, print) = open(&rig, &session, "plan.txt").await;

    // The agent's rewrite: same length, old modification time restored.
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "gamma beta\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(before)
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 11);

    let FilesResult::Stat {
        fingerprint: now, ..
    } = files(
        &rig,
        json!({"op": "stat", "sessionId": session, "file": file}),
    )
    .await
    .unwrap()
    else {
        panic!()
    };
    assert_ne!(
        now.as_deref(),
        Some(print.as_str()),
        "the poll would not see the rewrite"
    );

    let refused = write(&rig, &session, &file, "alpha beta\nmy line\n", &print).await;
    assert_eq!(
        refused.as_ref().map_err(|e| e.code),
        Err(ErrorCode::Stale),
        "{refused:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "gamma beta\n");
}
