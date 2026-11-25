//! App-restart persistence tests (FIX 3): a per-(chat, canonical profile) unclosed-window mark survives
//! `chat-profiles.json` reload into `AgentWindows` dropped marks and clears on full close or chat deletion.
//! Depends on `agent_close_incarnation_tests`, `agent_close_window_tests`, `edge_agent_tests::fixture`,
//! `gateway`, `profile_bindings`, and `test_support`.
use super::{
    agent_close_incarnation_tests::{ALL_CLOSED, close_chat_with, live_status, open_session},
    agent_close_window_tests::{
        MIXED_FAILURE, REPORTED_EARLIER, closes, observed_by_owner, profile_row,
    },
    edge_agent_tests, gateway, profile_bindings, test_support,
};
use serde_json::json;
use vw_platform::sync::LockRecover;

fn read_bindings_json(core: &crate::Core) -> serde_json::Value {
    let dir = core
        .shared
        .chrome_control
        .dir()
        .unwrap_or_else(|| panic!("the fixture has a data directory"));
    let bytes = std::fs::read(dir.join("chat-profiles.json"))
        .unwrap_or_else(|error| panic!("chat-profiles.json is readable: {error}"));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("chat-profiles.json is JSON: {error}"))
}

async fn open_and_remember_window(
    core: &crate::Core,
    profile: &str,
    session_id: &str,
    window: u64,
    incarnation: &str,
) {
    observed_by_owner(core, &profile_row(profile, incarnation));
    let profile_owned = profile.to_owned();
    let session_owned = session_id.to_owned();
    let inc_owned = incarnation.to_owned();
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => live_status(
            &session_owned,
            "s",
            &profile_owned,
            Some(window),
            &inc_owned,
        ),
        "chrome_session" if args["action"] == "open" => json!({
            "structuredContent": {"sessionId": session_owned, "profileId": profile_owned, "state": "active"}
        }),
        "chrome_execute" if args["action"] == "tabs.create" => {
            let win = if args["args"]["url"] == "https://example.org/" {
                window + 1
            } else {
                window
            };
            json!({
                "isError": false,
                "structuredContent": {"status": "completed", "data": {"tab": {"tabId": 71, "windowId": win}}}
            })
        }
        _ => json!({"structuredContent": {"status": "completed"}}),
    });
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action": "open", "profileId": profile}),
    )
    .await
    .unwrap_or_else(|error| panic!("open failed: {error}"));
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId": session_id, "action": "tabs.create", "args": {"url": "https://example.test/"}}),
    )
    .await
    .unwrap_or_else(|error| panic!("tabs.create failed: {error}"));
}

fn assert_noop_window_mark_sync(core: &crate::Core, chat: &str, profile: &str) {
    let Some(dir) = core.shared.chrome_control.dir() else {
        panic!("expected chrome_control dir");
    };
    let bindings_path = dir.join("chat-profiles.json");
    let raw = std::fs::read(&bindings_path)
        .unwrap_or_else(|error| panic!("read chat-profiles.json failed: {error}"));
    let mut doc: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&raw)
        .unwrap_or_else(|error| panic!("parse chat-profiles.json failed: {error}"));
    doc.insert("_noop_sentinel".into(), json!("untouched"));
    let encoded = serde_json::to_vec(&doc)
        .unwrap_or_else(|error| panic!("encode chat-profiles.json failed: {error}"));
    std::fs::write(&bindings_path, encoded)
        .unwrap_or_else(|error| panic!("write chat-profiles.json failed: {error}"));
    profile_bindings::sync_window_marks(&core.shared.chrome_control, chat, profile, true, true);
    let after = std::fs::read_to_string(&bindings_path)
        .unwrap_or_else(|error| panic!("re-read chat-profiles.json failed: {error}"));
    assert!(
        after.contains("\"_noop_sentinel\""),
        "no-op window mark sync with warned mark already persisted must not rewrite chat-profiles.json"
    );
}

#[tokio::test]
async fn fix3_app_restart_warns_once_and_clears_persisted_mark_on_live_close() {
    let fixture_name = "fix3-restart-warns-once";
    let profile = "chrome:AAAA1111-1111-4111-8111-111111111111";
    let profiles = json!([{"profileId": profile, "name": "Work", "connected": true}]);
    let core = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_and_remember_window(&core, profile, "s1", 7, "inc-1").await;
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", profile),
        vec![7]
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([super::profiles::canonical(profile)])
    );
    core.shutdown().await;

    // App restarts from the same data directory while helper keeps running and later opens window 9.
    let core2 = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_session(&core2, "s", "s2", profile, "active");
    observed_by_owner(&core2, &profile_row(profile, "inc-1"));
    let (first, sent1) = close_chat_with(
        &core2,
        "s",
        profile,
        live_status("s2", "s", profile, Some(9), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent1), vec![json!(9)]);
    assert!(first.to_string().contains(MIXED_FAILURE), "{first}");
    assert!(!first.to_string().contains(ALL_CLOSED), "{first}");
    assert!(
        read_bindings_json(&core2)
            .get("unclosed_windows")
            .and_then(|marks| marks.get("s"))
            .is_none(),
        "live close after restart must clear persisted mark"
    );
    assert_noop_window_mark_sync(&core2, "s", profile);

    // Second close closes only its own session's window and keeps the reported-earlier clause.
    open_session(&core2, "s", "s3", profile, "active");
    let (second, sent2) = close_chat_with(
        &core2,
        "s",
        profile,
        live_status("s3", "s", profile, Some(10), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent2), vec![json!(10)]);
    assert!(second.to_string().contains(ALL_CLOSED), "{second}");
    assert!(second.to_string().contains(REPORTED_EARLIER), "{second}");
    assert!(!second.to_string().contains(MIXED_FAILURE), "{second}");
    core2.shutdown().await;

    // A subsequent app restart still knows an earlier window was reported until the chat is deleted.
    let core3 = edge_agent_tests::fixture(fixture_name, profiles).await;
    open_session(&core3, "s", "s4", profile, "active");
    observed_by_owner(&core3, &profile_row(profile, "inc-1"));
    let (third, sent3) = close_chat_with(
        &core3,
        "s",
        profile,
        live_status("s4", "s", profile, Some(11), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent3), vec![json!(11)]);
    assert!(third.to_string().contains(ALL_CLOSED), "{third}");
    assert!(third.to_string().contains(REPORTED_EARLIER), "{third}");
    assert!(!third.to_string().contains(MIXED_FAILURE), "{third}");
    core3.shared.chrome_control.end_chat("s", true);
    assert!(
        read_bindings_json(&core3)
            .get("warned_windows")
            .and_then(|marks| marks.get("s"))
            .is_none(),
        "deleting a chat must clear its warned_windows mark"
    );
    test_support::chat(&core3.shared, "s");
    open_session(&core3, "s", "s5", profile, "active");
    let (after_delete, _) = close_chat_with(
        &core3,
        "s",
        profile,
        live_status("s5", "s", profile, Some(12), "inc-1"),
        &[],
    )
    .await;
    assert!(
        after_delete.to_string().contains(ALL_CLOSED),
        "{after_delete}"
    );
    assert!(
        !after_delete.to_string().contains(REPORTED_EARLIER),
        "{after_delete}"
    );
    core3.shutdown().await;
}

#[tokio::test]
async fn fix3_closed_windows_before_restart_give_no_warning_after_restart() {
    let fixture_name = "fix3-closed-before-restart";
    let profile = "chrome:work";
    let profiles = json!([{"profileId": profile, "name": "Work", "connected": true}]);
    let core = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_and_remember_window(&core, profile, "s1", 7, "inc-1").await;
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([profile])
    );

    let (closed_reply, sent) = close_chat_with(
        &core,
        "s",
        profile,
        live_status("s1", "s", profile, Some(7), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent), vec![json!(7)]);
    assert!(
        closed_reply.to_string().contains(ALL_CLOSED),
        "{closed_reply}"
    );
    assert!(
        read_bindings_json(&core)
            .get("unclosed_windows")
            .and_then(|marks| marks.get("s"))
            .is_none(),
        "confirming all windows closed must clear persisted mark"
    );
    core.shutdown().await;

    let core2 = edge_agent_tests::fixture(fixture_name, profiles).await;
    open_session(&core2, "s", "s2", profile, "active");
    observed_by_owner(&core2, &profile_row(profile, "inc-1"));
    let (after_restart, sent2) = close_chat_with(
        &core2,
        "s",
        profile,
        live_status("s2", "s", profile, Some(9), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent2), vec![json!(9)]);
    assert!(
        after_restart.to_string().contains(ALL_CLOSED),
        "{after_restart}"
    );
    assert!(
        !after_restart.to_string().contains(MIXED_FAILURE),
        "{after_restart}"
    );
    core2.shutdown().await;
}

#[tokio::test]
async fn fix3_deleted_chat_removes_persisted_mark_while_archive_keeps_it() {
    let profile = "chrome:work";
    let core = edge_agent_tests::fixture(
        "fix3-delete-removes-mark",
        json!([{"profileId": profile, "name": "Work", "connected": true}]),
    )
    .await;
    open_and_remember_window(&core, profile, "s1", 7, "inc-1").await;
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([profile])
    );

    core.shared.chrome_control.end_chat("s", false);
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([profile]),
        "closing/archiving a chat keeps its persisted mark"
    );

    core.shared.chrome_control.end_chat("s", true);
    assert!(
        read_bindings_json(&core)
            .get("unclosed_windows")
            .and_then(|marks| marks.get("s"))
            .is_none(),
        "deleting a chat must remove its persisted mark"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn fix3_no_live_session_after_restart_warns_and_keeps_mark_until_live_close() {
    let fixture_name = "fix3-restart-no-live-session";
    let profile = "edge:work";
    let profiles = json!([{"profileId": profile, "name": "Work", "connected": true}]);
    let core = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_and_remember_window(&core, profile, "s1", 7, "inc-1").await;
    core.shutdown().await;

    let core2 = edge_agent_tests::fixture(fixture_name, profiles).await;
    open_session(&core2, "s", "s1", profile, "disconnected");
    observed_by_owner(&core2, &profile_row(profile, "inc-1"));
    profile_bindings::mark_uncertain(&core2.shared.chrome_control, "s", None, Some(profile));
    let (offline_reply, sent) = close_chat_with(
        &core2,
        "s",
        profile,
        live_status("s1", "s", profile, None, "inc-1"),
        &[],
    )
    .await;
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        offline_reply.to_string().contains("ended by close_browser")
            && offline_reply.to_string().contains("may still be open"),
        "{offline_reply}"
    );
    assert_eq!(
        read_bindings_json(&core2)["unclosed_windows"]["s"],
        json!([profile]),
        "offline close with no live session keeps persisted mark"
    );

    open_session(&core2, "s", "s2", profile, "active");
    let (live_reply, sent_live) = close_chat_with(
        &core2,
        "s",
        profile,
        live_status("s2", "s", profile, Some(9), "inc-1"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_live), vec![json!(9)]);
    assert!(
        live_reply.to_string().contains(MIXED_FAILURE),
        "{live_reply}"
    );
    assert!(
        read_bindings_json(&core2)
            .get("unclosed_windows")
            .and_then(|marks| marks.get("s"))
            .is_none()
    );
    core2.shutdown().await;
}

#[tokio::test]
async fn fix3_old_and_new_files_load_and_noop_mark_changes_do_not_write() {
    // Old chat-profiles.json without `unclosed_windows` still loads cleanly.
    let old_dir = std::env::temp_dir().join(format!("fix3-old-file-{}", std::process::id()));
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::write(
        old_dir.join("chat-profiles.json"),
        b"{\"chats\":{\"s\":{\"chat\":\"s\",\"profile\":\"chrome:other\",\"owner\":null}},\"sessions\":{}}",
    )
    .unwrap();
    let loaded = profile_bindings::Bindings::load(Some(&old_dir));
    assert_eq!(
        loaded.chats().get("s").map(String::as_str),
        Some("chrome:other")
    );
    assert_eq!(loaded.unclosed_windows().count(), 0);
    let _ = std::fs::remove_dir_all(&old_dir);

    let fixture_name = "fix3-noop-writes";
    let profile = "chrome:work";
    let profiles = json!([{"profileId": profile, "name": "Work", "connected": true}]);
    let core = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_and_remember_window(&core, profile, "s1", 7, "inc-1").await;
    let bindings_path = core
        .shared
        .chrome_control
        .dir()
        .unwrap()
        .join("chat-profiles.json");

    // Plant a sentinel key in chat-profiles.json to detect any redundant file rewrite.
    let mut doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&bindings_path).unwrap()).unwrap();
    doc.insert("_noop_sentinel".into(), json!("untouched"));
    std::fs::write(&bindings_path, serde_json::to_vec(&doc).unwrap()).unwrap();

    // A second tabs.create (opening new window 8) leaves the mark unchanged -> must NOT write.
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId": "s1", "action": "tabs.create", "args": {"url": "https://example.org/"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", profile),
        vec![8, 7]
    );
    let raw_after_second = std::fs::read_to_string(&bindings_path).unwrap();
    assert!(
        raw_after_second.contains("\"_noop_sentinel\""),
        "second remembered window in same chat+profile must not rewrite chat-profiles.json: {raw_after_second}"
    );

    // Remembering with no known browser incarnation returns false and must NOT write.
    let unclosed_before = read_bindings_json(&core)["unclosed_windows"].clone();
    test_support::chat(&core.shared, "no-inc-chat");
    open_session(&core, "no-inc-chat", "s-no-inc", "edge:work", "active");
    let no_inc_args = json!({"action": "tabs.create", "sessionId": "s-no-inc"});
    let no_inc_res = json!({"isError": false, "structuredContent": {"status": "completed", "data": {"tab": {"tabId": 1, "windowId": 12}}}});
    super::agent_close::remember(
        &core.shared,
        "no-inc-chat",
        "chrome_execute",
        &no_inc_args,
        &no_inc_res,
    );
    assert!(
        std::fs::read_to_string(&bindings_path)
            .unwrap()
            .contains("\"_noop_sentinel\""),
        "remember without known incarnation must not rewrite chat-profiles.json"
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"],
        unclosed_before
    );

    // A file containing `unclosed_windows` loads into ChromeControl dropped marks on startup.
    let mut loaded_doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&bindings_path).unwrap()).unwrap();
    loaded_doc.insert(
        "unclosed_windows".into(),
        json!({"s": [profile], "loaded-chat": ["edge:work"]}),
    );
    std::fs::write(&bindings_path, serde_json::to_vec(&loaded_doc).unwrap()).unwrap();
    core.shutdown().await;

    let core2 = edge_agent_tests::fixture(fixture_name, profiles).await;
    assert!(
        core2
            .shared
            .chrome_control
            .agent_windows
            .lock_or_recover()
            .has_dropped("loaded-chat", "edge:work"),
        "loaded unclosed_windows must populate dropped marks at ChromeControl startup"
    );
    core2.shutdown().await;
}

#[tokio::test]
async fn fix3_second_chat_isolation_failed_window_close_and_clear_noop() {
    let fixture_name = "fix3-isolation-and-failure";
    let chrome = "chrome:work";
    let edge = "edge:work";
    let profiles = json!([
        {"profileId": chrome, "name": "Work", "connected": true},
        {"profileId": edge, "name": "EdgeWork", "connected": true}
    ]);
    let core = edge_agent_tests::fixture(fixture_name, profiles.clone()).await;
    open_and_remember_window(&core, chrome, "s1", 7, "chrome-inc").await;

    test_support::chat(&core.shared, "other");
    core.shared.access.observe_request(
        "session/agent/update",
        "other",
        &json!({"agentName": "auto-approve"}),
    );
    observed_by_owner(&core, &profile_row(edge, "edge-inc"));
    open_session(&core, "other", "o1", edge, "active");
    profile_bindings::remember(&core.shared.chrome_control, "other", edge, Some("o1")).unwrap();
    super::agent_close::remember(
        &core.shared,
        "other",
        "chrome_execute",
        &json!({"sessionId": "o1", "action": "tabs.create"}),
        &json!({"isError": false, "structuredContent": {"status": "completed", "data": {"tab": {"tabId": 91, "windowId": 15}}}}),
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([chrome])
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["other"],
        json!([edge])
    );

    // Failed window.close leaves live_failed = true -> persisted mark for "s" MUST stay!
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => live_status("s1", "s", chrome, Some(7), "chrome-inc"),
        "chrome_execute" if args["action"] == "window.close" => json!({"isError": true}),
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent": {"state": "stopped"}})
        }
        _ => json!({"structuredContent": {"status": "completed"}}),
    });
    let failed_reply = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action": "close_browser", "profileId": chrome}),
    )
    .await
    .unwrap();
    assert!(
        !failed_reply.to_string().contains(ALL_CLOSED),
        "{failed_reply}"
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["s"],
        json!([chrome]),
        "failed window.close must retain persisted mark"
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["other"],
        json!([edge])
    );

    // Clearing via clear_unclosed_window removes "s", leaves "other", and a second clear is a no-op write.
    profile_bindings::clear_unclosed_window(&core.shared.chrome_control, "s", chrome);
    assert!(
        read_bindings_json(&core)["unclosed_windows"]
            .get("s")
            .is_none()
    );
    assert_eq!(
        read_bindings_json(&core)["unclosed_windows"]["other"],
        json!([edge])
    );
    let bindings_path = core
        .shared
        .chrome_control
        .dir()
        .unwrap()
        .join("chat-profiles.json");
    let mut doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&bindings_path).unwrap()).unwrap();
    doc.insert("_noop_sentinel".into(), json!("untouched"));
    std::fs::write(&bindings_path, serde_json::to_vec(&doc).unwrap()).unwrap();
    profile_bindings::clear_unclosed_window(&core.shared.chrome_control, "s", chrome);
    assert!(
        std::fs::read_to_string(&bindings_path)
            .unwrap()
            .contains("\"_noop_sentinel\""),
        "clearing an absent mark must not rewrite chat-profiles.json"
    );
    core.shutdown().await;
}
