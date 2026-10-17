//! A session whose folder is reached through a symlinked folder (`linked -> repo`, the way a macOS temporary
//! folder is reached as `/var/...` while it is `/private/var/...`). git names the repository root by the real
//! folder; the session and its agent name files through the link. The Changes view must treat both spellings as
//! one folder: the turn label comes from the agent's file effect, and `folderSessions` finds the session under
//! either spelling of the folder.

// Unix only: the fixture creates a symlinked folder. The skip is listed by name in `platform_skips.rs`.
#![cfg(unix)]

mod changes;
mod common;

use std::path::Path;
use std::time::Duration;

use changes::TempRepo;
use common::core::{ScriptRig, script_core_with};
use serde_json::{Value, json};
use vw_core_api::{
    ChangesMode, ChangesResult, ChangesView, Command, CommandResult, CoreError, CoreEvent, Origin,
};

async fn rig(name: &str, data: &Path) -> ScriptRig {
    let data = data.to_path_buf();
    script_core_with(name, &["--status-events", "--edit-files"], move |config| {
        config.changes_dir = Some(data);
    })
    .await
}

async fn exec(rig: &ScriptRig, value: Value) -> Result<CommandResult, CoreError> {
    rig.core
        .execute(
            Origin::Ui,
            serde_json::from_value::<Command>(value).unwrap(),
        )
        .await
}

/// Starts a session in `folder` (picked by the user) whose agent may edit files.
async fn start(rig: &ScriptRig, folder: &Path) -> String {
    rig.core.register_picked_folder(folder).unwrap();
    let result = exec(
        rig,
        json!({"type": "startSession", "agentConfig": {"cwd": folder.to_str().unwrap()}}),
    )
    .await
    .unwrap();
    let CommandResult::Session { snapshot, .. } = result else {
        panic!("{result:?}")
    };
    let sid = snapshot.session_id;
    exec(
        rig,
        json!({"type": "setAccessMode", "sessionId": sid, "agentName": "accept-edits"}),
    )
    .await
    .unwrap();
    sid
}

async fn read(rig: &ScriptRig, sid: &str) -> ChangesView {
    match exec(
        rig,
        json!({"type": "changes", "request": {"op": "read", "sessionId": sid}}),
    )
    .await
    {
        Ok(CommandResult::Changes {
            result: ChangesResult::View { view },
        }) => view,
        other => panic!("{other:?}"),
    }
}

/// Runs a turn whose agent writes the `edit:<path>` files, and waits until the core reports the changes.
async fn turn_with_edits(rig: &ScriptRig, sid: &str, text: &str) {
    let mut events = rig.core.subscribe(Some(sid.to_owned()));
    exec(
        rig,
        json!({"type": "startTurn", "sessionId": sid, "message": [{"type": "text", "text": text}]}),
    )
    .await
    .unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.next().await {
            if matches!(event, CoreEvent::ChangesUpdated { .. }) {
                return;
            }
        }
    })
    .await;
    assert!(waited.is_ok(), "no changesUpdated event within 60 s");
}

/// Starts a session in the repository's real folder or through the link, lets its agent edit `a.txt`, then checks
/// the turn label and `folderSessions` under both spellings, and that another repository does not list it.
async fn effect_labels_and_folder_lookup(name: &str, through_link: bool) {
    let repo = TempRepo::with_files(&[("a.txt", "one\ntwo\n")]);
    let other = TempRepo::with_files(&[("b.txt", "b\n")]);
    let link = repo.base.join("linked");
    std::os::unix::fs::symlink(&repo.root, &link).unwrap();
    let rig = rig(name, &repo.data).await;
    let sid = start(&rig, if through_link { &link } else { &repo.root }).await;

    turn_with_edits(&rig, &sid, "please edit:a.txt").await;
    let view = read(&rig, &sid).await;
    assert_eq!(view.mode, ChangesMode::Git, "{view:?}");
    let file = view
        .files
        .iter()
        .find(|f| f.path == "a.txt")
        .expect("a.txt changed");
    let turn = file.turn.as_ref().expect("a turn label");
    assert_eq!(turn.number, 1);
    assert_eq!(
        turn.source,
        vw_changes::TurnSource::Effect,
        "the turn label comes from the agent's file effect"
    );

    for (folder, listed) in [(&repo.root, true), (&link, true), (&other.root, false)] {
        let found = exec(
            &rig,
            json!({"type": "changes", "request": {"op": "folderSessions", "cwd": folder.to_str().unwrap()}}),
        )
        .await
        .unwrap();
        let CommandResult::Changes {
            result: ChangesResult::Folder { is_git, sessions },
        } = found
        else {
            panic!("{found:?}")
        };
        assert!(is_git);
        let expected: Vec<&str> = if listed {
            vec![sid.as_str()]
        } else {
            Vec::new()
        };
        assert_eq!(
            sessions
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            expected,
            "folderSessions for {}",
            folder.display()
        );
    }
    rig.core.shutdown().await;
}

#[tokio::test]
async fn session_through_a_linked_folder_keeps_effect_labels_and_is_found_by_folder() {
    effect_labels_and_folder_lookup("changes-linked-folder", true).await;
}

/// The control: the same checks for a session started in the real folder.
#[tokio::test]
async fn session_in_the_real_folder_keeps_effect_labels_and_is_found_by_folder() {
    effect_labels_and_folder_lookup("changes-real-folder", false).await;
}
