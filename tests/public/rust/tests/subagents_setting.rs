//! Subagent settings over a scripted fake Vibe server.
//! PREDICT: writes use a short-lived chat and leave the user chat usable.

mod common;

use std::path::Path;

use common::subagents_rig::{
    listed, named, position, record, set_subagents, setup, temporary_ids, users_chat_in,
};
use serde_json::{Value, json};
use vw_core_api::{Command, CommandResult, Origin, SessionToolsResult};

#[tokio::test]
async fn the_value_is_written_through_a_short_lived_chat_and_the_users_chat_is_untouched() {
    let s = setup("sub-t2", &[]).await;
    let r = &s.r;
    let chat = users_chat_in(r, "/work/alpha").await;
    let before = listed(r).await;

    let result = set_subagents(r, false).await.unwrap();
    assert_eq!(
        result,
        SessionToolsResult::SubagentsSet {
            enabled: Some(false),
            leftover_session_id: None
        }
    );

    // Exactly one write, and not through the user's chat.
    let writes = named(r, "config/write");
    assert_eq!(writes.len(), 1, "{:?}", r.log());
    let helper = writes[0].clone();
    assert_ne!(helper, chat);
    assert!(
        r.on("config/write", &chat).is_empty(),
        "the user's chat got a config/write"
    );
    // The chat is started first, then written to and read through, then deleted.
    let start = r
        .log()
        .iter()
        .rposition(|(_, m, _)| m == "session/start")
        .expect("a start");
    let write = position(r, "config/write", &helper);
    let read = position(r, "config/fields/read", &helper);
    let delete = position(r, "session/delete", &helper);
    assert!(
        start < write && write < read && read < delete,
        "{:?}",
        r.log()
    );

    // Nothing is left: the list is what it was, nothing is temporary, the record is empty.
    assert_eq!(listed(r).await, before);
    assert!(temporary_ids(r).await.is_empty());
    assert!(record(&s.sessions_dir).is_empty());

    // The user's chat still runs a turn, and a new chat starts.
    r.begin_turn(&chat).await;
    assert_ne!(r.start().await, chat);

    // The saved value reads back as written.
    assert_eq!(
        set_subagents(r, true).await.unwrap(),
        SessionToolsResult::SubagentsSet {
            enabled: Some(true),
            leftover_session_id: None
        }
    );
    r.rig.core.shutdown().await;
}

#[tokio::test]
async fn the_short_lived_chat_opens_in_the_servers_own_folder() {
    let s = setup("sub-t6", &[]).await;
    let r = &s.r;
    let chat = users_chat_in(r, "/work/alpha").await;
    set_subagents(r, true).await.unwrap();

    // Every start the fake saw: the user's chat in its folder, then the short-lived one in the server's folder.
    let starts: Vec<Value> = std::fs::read_to_string(&s.agent_log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["method"] == "session/start")
        .collect();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert_eq!(starts[0]["agentConfig"]["cwd"], "/work/alpha");
    let helper_folder = starts[1]["agentConfig"]["cwd"].as_str().unwrap();
    assert_eq!(Path::new(helper_folder), r.rig.dir.as_path());
    assert_ne!(helper_folder, "/work/alpha");
    assert!(r.on("config/write", &chat).is_empty());
    r.rig.core.shutdown().await;
}

#[tokio::test]
async fn a_refused_write_is_an_error_and_the_short_lived_chat_is_still_deleted() {
    let s = setup("sub-t3", &["--subagents-reject"]).await;
    let r = &s.r;
    let before = listed(r).await;
    let error = set_subagents(r, true).await.unwrap_err();
    assert_eq!(error.message, "Vibe did not accept the change.");
    let helper = named(r, "config/write").remove(0);
    assert_eq!(named(r, "session/delete"), [helper]);
    assert_eq!(listed(r).await, before);
    assert!(temporary_ids(r).await.is_empty());
    assert!(record(&s.sessions_dir).is_empty());
    r.rig.core.shutdown().await;
}

#[tokio::test]
async fn a_delete_that_fails_once_is_tried_once_more() {
    let s = setup("sub-t4", &["--delete-faults", "1"]).await;
    let r = &s.r;
    let before = listed(r).await;
    let result = set_subagents(r, false).await.unwrap();
    assert_eq!(
        result,
        SessionToolsResult::SubagentsSet {
            enabled: Some(false),
            leftover_session_id: None
        }
    );
    let helper = named(r, "config/write").remove(0);
    assert_eq!(
        named(r, "session/delete"),
        [helper.clone(), helper],
        "one failed delete and one retry"
    );
    assert_eq!(listed(r).await, before);
    assert!(temporary_ids(r).await.is_empty());
    assert!(record(&s.sessions_dir).is_empty());
    r.rig.core.shutdown().await;
}

#[tokio::test]
async fn a_chat_that_cannot_be_deleted_is_named_and_removed_when_the_app_quits() {
    let s = setup("sub-t5", &["--delete-faults", "2"]).await;
    let r = &s.r;
    let result = set_subagents(r, false).await.unwrap();
    let helper = named(r, "config/write").remove(0);
    assert_eq!(
        result,
        SessionToolsResult::SubagentsSet {
            enabled: Some(false),
            leftover_session_id: Some(helper.clone())
        },
        "the value is saved and the leftover chat is named"
    );
    // It is visible: in the list, and recorded as temporary (the window shows it with the temporary chip).
    assert!(listed(r).await.contains(&helper));
    assert_eq!(temporary_ids(r).await, std::slice::from_ref(&helper));
    assert_eq!(record(&s.sessions_dir), std::slice::from_ref(&helper));
    assert_eq!(named(r, "session/delete").len(), 2);

    r.rig.core.shutdown().await;
    assert_eq!(
        named(r, "session/delete"),
        [helper.clone(), helper.clone(), helper],
        "quitting deletes it"
    );
    assert!(record(&s.sessions_dir).is_empty());
}

#[tokio::test]
async fn the_debug_api_origin_gets_the_same_result() {
    let s = setup("sub-origin", &[]).await;
    let r = &s.r;
    let command: Command = serde_json::from_value(
        json!({"type": "sessionTools", "request": {"op": "setSubagents", "enabled": true}}),
    )
    .unwrap();
    let answer = r.rig.core.execute(Origin::DebugApi, command).await.unwrap();
    assert!(matches!(
        answer,
        CommandResult::SessionTools {
            result: SessionToolsResult::SubagentsSet {
                enabled: Some(true),
                leftover_session_id: None
            }
        }
    ));
    r.rig.core.shutdown().await;
}
