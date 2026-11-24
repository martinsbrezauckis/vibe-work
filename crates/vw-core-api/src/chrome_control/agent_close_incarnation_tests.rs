//! Browser-incarnation and memory-only window-tracking tests (`DBK17S-01` and `DBK17T-01`) for `close_browser`.
//! Depends on `agent_close_window_tests` helpers, `agent_firefox_close_tests::fixture`, `gateway`, `profile_bindings`,
//! and `test_support`.
use super::{
    agent_close_window_tests::{
        MIXED_FAILURE, closed, closes, created_by, created_in, observed, observed_by_owner,
        profile_row, status, status_with, stopped, two_chats,
    },
    agent_firefox_close_tests::fixture,
    gateway, profile_bindings, test_support,
};
use crate::{Origin, ask::tool_calls};
use serde_json::{Value, json};
use vw_platform::sync::LockRecover;

pub(super) const ALL_CLOSED: &str = "the agent's own tabs were closed";

pub(super) fn open_session(
    core: &crate::Core,
    chat: &str,
    session: &str,
    profile: &str,
    state: &str,
) {
    let chrome = &core.shared.chrome_control;
    chrome
        .tasks
        .lock_or_recover()
        .insert(session.into(), chat.into());
    chrome
        .work
        .lock_or_recover()
        .open(session, chat, state, profile, None);
}

fn bind_session(core: &crate::Core, chat: &str, session: &str, profile: &str) {
    assert!(
        profile_bindings::remember(&core.shared.chrome_control, chat, profile, Some(session))
            .is_ok()
    );
    open_session(core, chat, session, profile, "active");
}

pub(super) fn live_status(
    session: &str,
    chat: &str,
    profile: &str,
    window: Option<u64>,
    incarnation: &str,
) -> Value {
    json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":session,"clientSessionId":chat,"profileId":profile,"state":"active","windowId":window}],
        "profiles":[{"profileId":profile,"connected":true,"browserSessionId":incarnation,"extensionVersion":"0.1.15","features":["window.close"]}]}})
}

/// Older builds saved the remembered windows in chat-profiles.json (a `windows` field). That file must still load with its
/// choices intact and its windows ignored, and the next save leaves the field out: the windows are memory only (DBK17S-01).
#[tokio::test]
async fn an_older_bindings_file_with_saved_windows_still_loads_and_no_window_is_saved() {
    let (core, _) = fixture("agent-chrome-close-old-file").await;
    two_chats(&core);
    let chrome = &core.shared.chrome_control;
    let file = chrome
        .dir()
        .expect("a state folder")
        .join("chat-profiles.json");
    let old = json!({"chats":{"s":{"profile":"chrome:work","owner":null}},"sessions":{},
        "windows":[["s",[["chrome:work",7]]]]});
    std::fs::write(&file, old.to_string()).unwrap();
    let saved = profile_bindings::Bindings::load(chrome.dir());
    assert_eq!(
        saved.chats().get("s").map(String::as_str),
        Some("chrome:work"),
        "the older file loads with its choices"
    );
    assert!(
        saved.check("s", "chrome:work").is_ok(),
        "the file is readable"
    );
    *chrome.chat_profiles.lock_or_recover() = saved.chats();
    *chrome.bindings.lock_or_recover() = saved;
    assert!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work").is_empty(),
        "a saved window of an older build is not remembered"
    );
    created_in(&core, 8);
    assert!(profile_bindings::remember(chrome, "s", "chrome:work", Some("old-chrome")).is_ok());
    let text = std::fs::read_to_string(&file).unwrap();
    let written: Value = serde_json::from_str(&text).unwrap();
    assert!(
        written.get("windows").is_none(),
        "no window is saved: {text}"
    );
    assert!(
        written["chats"]["s"].is_object(),
        "the choice is saved: {text}"
    );
    core.shutdown().await;
}

/// `close_browser` of chat "s" in `chrome:work` against a live helper status, every `window.close` answered with the
/// given final inventory; returns the answer and what the app sent.
pub(super) async fn close_with(
    core: &crate::Core,
    live: Value,
    inventory: &'static [(u64, u64, bool)],
) -> (Value, Vec<(String, Value)>) {
    close_chat_with(core, "s", "chrome:work", live, inventory).await
}

pub(super) async fn close_chat_with(
    core: &crate::Core,
    chat: &str,
    profile: &str,
    live: Value,
    inventory: &'static [(u64, u64, bool)],
) -> (Value, Vec<(String, Value)>) {
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(inventory),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live.clone(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let answer = gateway::call(
        &core.shared,
        chat,
        "chrome_session",
        json!({"action":"close_browser","profileId":profile}),
    )
    .await
    .unwrap_or_else(|error| panic!("close_browser failed for {chat}/{profile}: {error}"));
    let sent = calls.lock_or_recover().clone();
    (answer, sent)
}

/// DBK17S-01: window 9 was the agent's in browser incarnation A. The browser restarted (incarnation B reuses the number 9
/// for a person's window, where the person may have moved an agent tab and handed the window over): the helper's status
/// now names B. The remembered 9 is not nominated, only the new session's own window 10 is, and the agent is told that
/// tabs may stay open. PREDICT (red on 5b516c51e): `window.close` was sent for 10 and then for the stale 9.
#[tokio::test]
async fn a_window_number_from_an_earlier_browser_incarnation_is_never_nominated() {
    let (core, _) = fixture("agent-chrome-close-reused-number").await;
    two_chats(&core);
    created_in(&core, 9);
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![9]
    );
    let mut live = status_with(Some(10));
    live["structuredContent"]["profiles"][0]["browserSessionId"] = json!("browser-b");
    let (answer, sent) = close_with(&core, live, &[(91, 9, true), (81, 8, false)]).await;
    assert_eq!(
        closes(&sent),
        vec![json!(10)],
        "only the live session's own window, never the stale number 9: {sent:?}"
    );
    assert!(
        answer.to_string().contains("could not be closed"),
        "{answer}"
    );
    assert!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work").is_empty(),
        "the window of the earlier incarnation is dropped"
    );
    core.shutdown().await;
}

/// The helper's status names no browser incarnation for the profile (an older helper, or a row without one): a remembered
/// window cannot be tied to the live browser, so it is not nominated, and the agent is told.
#[tokio::test]
async fn a_remembered_window_is_not_nominated_when_the_helper_names_no_incarnation() {
    let (core, _) = fixture("agent-chrome-close-no-incarnation").await;
    two_chats(&core);
    created_in(&core, 9);
    let mut live = status_with(None);
    live["structuredContent"]["profiles"][0]
        .as_object_mut()
        .unwrap()
        .remove("browserSessionId");
    let (answer, sent) = close_with(&core, live, &[(91, 9, true), (81, 8, false)]).await;
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        answer.to_string().contains("could not be closed"),
        "{answer}"
    );
    assert!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work").is_empty(),
        "an unverifiable window is dropped"
    );
    core.shutdown().await;
}

/// The same browser incarnation keeps its remembered windows: they are nominated as before (C17J J1/J2), also after the
/// observer read the same incarnation again, and an unconfirmed close stays remembered for a retry (DBK17R-02).
#[tokio::test]
async fn a_remembered_window_of_the_same_browser_incarnation_is_still_nominated() {
    let (core, _) = fixture("agent-chrome-close-same-incarnation").await;
    two_chats(&core);
    created_in(&core, 7);
    observed(&core, &status()["structuredContent"]);
    observed_by_owner(&core, &profile_row("chrome:work", "browser-a"));
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7],
        "the same incarnation observed again forgets nothing"
    );
    let (answer, sent) = close_with(
        &core,
        status_with(Some(9)),
        &[(71, 7, true), (91, 9, true), (81, 8, false)],
    )
    .await;
    assert_eq!(
        closes(&sent),
        vec![json!(9), json!(7)],
        "the live window, then the remembered one: {sent:?}"
    );
    assert!(answer.to_string().contains(ALL_CLOSED), "{answer}");
    core.shutdown().await;
}

/// A browser restart seen by the app's status observer drops the windows remembered in that profile, for every chat; the
/// windows of another profile, and a status that repeats the incarnation, change nothing.
/// PREDICT (red on 5b516c51e): a restarted browser kept every remembered window.
#[tokio::test]
async fn a_browser_restart_seen_by_the_observer_drops_that_profiles_windows() {
    let (core, _) = fixture("agent-chrome-close-observer").await;
    two_chats(&core);
    // A third chat in a second profile, with its own incarnation (a chat holds one live session at a time).
    observed(&core, &profile_row("chrome:other", "other-a"));
    test_support::chat(&core.shared, "third");
    core.shared.chrome_control.work.lock_or_recover().open(
        "other-session",
        "third",
        "active",
        "chrome:other",
        None,
    );
    created_in(&core, 7);
    created_by(&core, "other", "other-chrome", 8);
    created_by(&core, "third", "other-session", 5);
    let remembered =
        |chat: &str, profile: &str| super::agent_close::remembered(&core.shared, chat, profile);
    assert_eq!(remembered("s", "chrome:work"), vec![7]);
    assert_eq!(remembered("other", "chrome:work"), vec![8]);
    assert_eq!(remembered("third", "chrome:other"), vec![5]);
    // The same incarnation again: nothing is dropped.
    observed_by_owner(&core, &profile_row("chrome:work", "browser-a"));
    assert_eq!(remembered("s", "chrome:work"), vec![7]);
    // A row without an incarnation says nothing about it.
    observed_by_owner(&core, &profile_row("chrome:work", ""));
    assert_eq!(remembered("s", "chrome:work"), vec![7]);
    // The browser of chrome:work restarted.
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));
    assert!(remembered("s", "chrome:work").is_empty(), "chat s");
    assert!(remembered("other", "chrome:work").is_empty(), "every chat");
    assert_eq!(
        remembered("third", "chrome:other"),
        vec![5],
        "another profile"
    );
    // A window the agent opens now belongs to the new incarnation and is nominated in it.
    created_in(&core, 9);
    assert_eq!(remembered("s", "chrome:work"), vec![9]);
    let mut live = status_with(None);
    live["structuredContent"]["profiles"][0]["browserSessionId"] = json!("browser-b");
    let (answer, sent) = close_with(&core, live, &[(91, 9, true), (81, 8, false)]).await;
    assert_eq!(closes(&sent), vec![json!(9)], "{answer} {sent:?}");
    core.shutdown().await;
}

/// Without a browser incarnation known at remember time nothing is remembered: before any status named one, and for a row
/// that names none.
#[tokio::test]
async fn no_window_is_remembered_before_the_profiles_browser_incarnation_is_known() {
    let (core, _) = fixture("agent-chrome-close-unknown-incarnation").await;
    two_chats(&core);
    observed(&core, &profile_row("chrome:blank", ""));
    for profile in ["chrome:unseen", "chrome:blank"] {
        let session = format!("{profile}-session");
        core.shared
            .chrome_control
            .work
            .lock_or_recover()
            .open(&session, "s", "active", profile, None);
        created_by(&core, "s", &session, 6);
        assert!(
            super::agent_close::remembered(&core.shared, "s", profile).is_empty(),
            "nothing remembered in {profile}"
        );
    }
    created_in(&core, 7);
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7],
        "a profile whose incarnation is known remembers"
    );
    core.shutdown().await;
}

/// Only the chat's own `tabs.create` answer adds a window: a tab listing, a refused create and a create in the agent's
/// Firefox (closed through Settings' path, not by window) remember nothing, so nothing of them is ever nominated.
#[tokio::test]
async fn only_the_chats_own_tab_create_answers_add_a_remembered_window() {
    let (core, _) = fixture("agent-chrome-close-only-create").await;
    two_chats(&core);
    observed(&core, &profile_row("firefox:agent", "firefox-a"));
    core.shared.chrome_control.work.lock_or_recover().open(
        "agent-firefox",
        "s",
        "active",
        "firefox:agent",
        None,
    );
    let answer = |window: u64, error: bool| {
        json!({"isError":error,"structuredContent":{"status":"completed",
            "data":{"tab":{"tabId":window * 10 + 1,"windowId":window},
                "tabs":[{"tabId":window * 10 + 2,"windowId":window}]}}})
    };
    let remember = |session: &str, action: &str, result: Value| {
        super::agent_close::remember(
            &core.shared,
            "s",
            "chrome_execute",
            &json!({"sessionId":session,"action":action}),
            &result,
        );
    };
    remember("old-chrome", "tabs.list", answer(5, false));
    remember("old-chrome", "tabs.create", answer(6, true));
    remember("agent-firefox", "tabs.create", answer(4, false));
    for profile in ["chrome:work", "firefox:agent"] {
        assert_eq!(
            super::agent_close::remembered(&core.shared, "s", profile),
            Vec::<u64>::new(),
            "nothing remembered in {profile}"
        );
    }
    core.shutdown().await;
}

/// DBK17T-01: window 7 remembered in incarnation A, observer sees incarnation B (dropping 7), reopened session closes
/// window 9 -> must warn that earlier tabs could not be closed; a second `close_browser` (no new drop) answers normally.
#[tokio::test]
async fn dbk17t_observer_drop_must_report_older_tabs_may_remain() {
    let (core, _) = fixture("dbk17t-observer-drop-text").await;
    two_chats(&core);
    created_in(&core, 7);
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        [] as [u64; 0]
    );

    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (answer, sent) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", Some(9), "browser-b"),
        &[],
    )
    .await;
    let text = answer.to_string();
    assert_eq!(closes(&sent), vec![json!(9)]);
    assert!(text.contains("could not be closed"), "{answer}");
    assert!(text.contains(MIXED_FAILURE), "{answer}");
    assert!(!text.contains(ALL_CLOSED), "{answer}");
    assert!(!text.contains("the person"), "{answer}");

    open_session(&core, "s", "third-chrome", "chrome:work", "active");
    let (second, sent_second) = close_with(
        &core,
        live_status("third-chrome", "s", "chrome:work", Some(10), "browser-b"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_second), vec![json!(10)]);
    assert!(second.to_string().contains(ALL_CLOSED), "{second}");
    core.shutdown().await;
}

/// When the reopened session has no own window (`windowId: null`), an observer drop still triggers the warning once
/// and clears after that answer.
#[tokio::test]
async fn observer_drop_with_no_new_window_warns_then_clears() {
    let (core, _) = fixture("dbk17t-observer-no-new-window").await;
    two_chats(&core);
    created_in(&core, 7);
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));

    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (first, sent_first) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", None, "browser-b"),
        &[],
    )
    .await;
    let text = first.to_string();
    assert!(closes(&sent_first).is_empty(), "{sent_first:?}");
    assert!(text.contains("could not be closed"), "{first}");
    assert!(
        text.contains("tell the user they may still be open"),
        "{first}"
    );
    assert!(!text.contains("the person"), "{first}");

    open_session(&core, "s", "third-chrome", "chrome:work", "active");
    let (second, _) = close_with(
        &core,
        live_status("third-chrome", "s", "chrome:work", None, "browser-b"),
        &[],
    )
    .await;
    assert!(
        !second.to_string().contains("could not be closed"),
        "{second}"
    );
    core.shutdown().await;
}

/// When no live session exists after an observer drop, the mark counts as an earlier window (`!attempted && earlier`)
/// and stays until a reopened session performs `close_browser`.
#[tokio::test]
async fn observer_drop_without_live_session_counts_as_earlier_window() {
    let (core, _) = fixture("dbk17t-observer-no-live").await;
    two_chats(&core);
    created_in(&core, 7);
    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));

    let (ended, sent_ended) = close_with(
        &core,
        live_status("old-chrome", "s", "chrome:work", None, "browser-b"),
        &[],
    )
    .await;
    let ended_text = ended.to_string();
    assert!(closes(&sent_ended).is_empty(), "{sent_ended:?}");
    assert!(
        ended_text.contains("open a new browser session and use close_browser again"),
        "{ended}"
    );
    assert!(ended_text.contains("tell the user"), "{ended}");

    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (stopped_answer, sent_stopped) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", Some(9), "browser-b"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_stopped), vec![json!(9)]);
    assert!(
        stopped_answer.to_string().contains(MIXED_FAILURE),
        "{stopped_answer}"
    );

    open_session(&core, "s", "third-chrome", "chrome:work", "active");
    let (clean, _) = close_with(
        &core,
        live_status("third-chrome", "s", "chrome:work", Some(10), "browser-b"),
        &[],
    )
    .await;
    assert!(clean.to_string().contains(ALL_CLOSED), "{clean}");
    core.shutdown().await;
}

/// A deleted chat drops its mark while an ordinary stop (`deleted = false`), another chat on the same profile, and
/// a chat on a different profile keep their state.
#[tokio::test]
async fn deleted_chat_drops_observer_mark_while_other_chats_keep_theirs() {
    let (core, _) = fixture("dbk17t-observer-delete").await;
    two_chats(&core);
    observed_by_owner(&core, &profile_row("chrome:personal", "personal-a"));
    test_support::chat(&core.shared, "third");
    bind_session(&core, "third", "other-session", "chrome:personal");

    created_in(&core, 7);
    created_by(&core, "other", "other-chrome", 8);
    created_by(&core, "third", "other-session", 5);

    observed_by_owner(&core, &profile_row("chrome:work", ""));
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7]
    );

    observed_by_owner(
        &core,
        &json!({"profiles":[
            {"profileId":"chrome:work","connected":true,"browserSessionId":"browser-b"},
            {"profileId":"chrome:personal","connected":true,"browserSessionId":"personal-a"}
        ]}),
    );
    let chrome = &core.shared.chrome_control;
    chrome.end_chat("other", false);
    chrome.end_chat("s", true);

    test_support::chat(&core.shared, "s");
    open_session(&core, "s", "s-reborn", "chrome:work", "active");
    let (s_answer, _) = close_with(
        &core,
        live_status("s-reborn", "s", "chrome:work", Some(9), "browser-b"),
        &[],
    )
    .await;
    assert!(
        s_answer.to_string().contains(ALL_CLOSED),
        "deleted chat must drop its mark: {s_answer}"
    );

    open_session(&core, "other", "other-reopened", "chrome:work", "active");
    let (other_answer, _) = close_chat_with(
        &core,
        "other",
        "chrome:work",
        live_status(
            "other-reopened",
            "other",
            "chrome:work",
            Some(11),
            "browser-b",
        ),
        &[],
    )
    .await;
    assert!(
        other_answer.to_string().contains(MIXED_FAILURE),
        "stopped chat must keep its mark: {other_answer}"
    );

    let (third_answer, third_sent) = close_chat_with(
        &core,
        "third",
        "chrome:personal",
        live_status(
            "other-session",
            "third",
            "chrome:personal",
            Some(12),
            "personal-a",
        ),
        &[(51, 5, true)],
    )
    .await;
    assert_eq!(closes(&third_sent), vec![json!(12), json!(5)]);
    assert!(
        third_answer.to_string().contains(ALL_CLOSED),
        "{third_answer}"
    );
    core.shutdown().await;
}

/// The chat cap (`MAX_CHATS = 256`) and per-chat profile cap (`MAX_PER_CHAT = 16`) hold for observer-drop marks,
/// evicting the oldest entry first.
#[tokio::test]
async fn observer_drop_marks_respect_chat_and_per_chat_caps() {
    let (core, _) = fixture("dbk17t-observer-caps").await;
    two_chats(&core);
    for i in 0..256u64 {
        created_by(&core, &format!("cap{i}"), "old-chrome", 1000 + i);
    }
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));

    // A second profile mark on an already-marked chat (`cap1`) consumes no new chat slot.
    observed_by_owner(&core, &profile_row("chrome:personal", "personal-a"));
    open_session(&core, "cap1", "other-sess", "chrome:personal", "active");
    created_by(&core, "cap1", "other-sess", 3001);
    observed_by_owner(&core, &profile_row("chrome:personal", "personal-b"));

    // A 257th marked chat (`cap256`) evicts the oldest marked chat (`cap0`).
    created_by(&core, "cap256", "old-chrome", 1256);
    observed_by_owner(&core, &profile_row("chrome:work", "browser-c"));

    for (chat, expect_warn) in [
        ("cap0", false),
        ("cap1", true),
        ("cap255", true),
        ("cap256", true),
    ] {
        test_support::chat(&core.shared, chat);
        let sess = format!("reopen-{chat}");
        open_session(&core, chat, &sess, "chrome:work", "active");
        let (answer, _) = close_chat_with(
            &core,
            chat,
            "chrome:work",
            live_status(&sess, chat, "chrome:work", Some(90), "browser-c"),
            &[],
        )
        .await;
        assert_eq!(
            answer.to_string().contains(MIXED_FAILURE),
            expect_warn,
            "{chat} (expect_warn={expect_warn}): {answer}"
        );
    }

    // Per-chat profile cap (16): marking 17 distinct profiles on one chat evicts the oldest (`chrome:p0`).
    test_support::chat(&core.shared, "many");
    for p in 0..17u64 {
        let profile = format!("chrome:p{p}");
        let sess = format!("many-s{p}");
        super::profiles::paired(&core.shared.chrome_control, &profile, None);
        observed_by_owner(&core, &profile_row(&profile, "inc-a"));
        open_session(&core, "many", &sess, &profile, "active");
        created_by(&core, "many", &sess, 500 + p);
        observed_by_owner(&core, &profile_row(&profile, "inc-b"));
    }
    let all_p: Vec<Value> = (0..17u64)
        .map(|p| {
            json!({"profileId":format!("chrome:p{p}"),"connected":true,"browserSessionId":"inc-b"})
        })
        .collect();
    observed_by_owner(&core, &json!({"profiles":all_p}));
    for (p, expect_warn) in [(0, false), (1, true), (16, true)] {
        let profile = format!("chrome:p{p}");
        let sess = format!("many-close-{p}");
        bind_session(&core, "many", &sess, &profile);
        let (answer, _) = close_chat_with(
            &core,
            "many",
            &profile,
            live_status(&sess, "many", &profile, Some(80 + p), "inc-b"),
            &[],
        )
        .await;
        assert_eq!(
            answer.to_string().contains(MIXED_FAILURE),
            expect_warn,
            "{profile}: {answer}"
        );
    }
    core.shutdown().await;
}

/// Edge, mixed-case Chrome UUIDs, and `ask` / `accept-edits` / `auto-approve` access modes preserve the mark until a
/// real `close_browser` answer runs.
#[tokio::test]
async fn edge_uuid_and_access_modes_preserve_and_clear_observer_mark() {
    let (core, _) = fixture("dbk17t-observer-variants").await;
    let upper_uuid = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower_uuid = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    super::profiles::paired(&core.shared.chrome_control, lower_uuid, None);
    observed_by_owner(&core, &profile_row(lower_uuid, "u-a"));
    bind_session(&core, "s", "uuid-s1", upper_uuid);
    created_by(&core, "s", "uuid-s1", 7);
    observed_by_owner(&core, &profile_row(lower_uuid, "u-b"));

    open_session(&core, "s", "uuid-s2", upper_uuid, "active");
    let (uuid_answer, _) = close_chat_with(
        &core,
        "s",
        upper_uuid,
        live_status("uuid-s2", "s", lower_uuid, Some(9), "u-b"),
        &[],
    )
    .await;
    assert!(
        uuid_answer.to_string().contains(MIXED_FAILURE),
        "{uuid_answer}"
    );

    // Edge across Ask refusal -> Accept edits -> Full access follow-up.
    observed_by_owner(&core, &profile_row("edge:work", "edge-a"));
    bind_session(&core, "s", "edge-s1", "edge:work");
    created_by(&core, "s", "edge-s1", 7);
    observed_by_owner(&core, &profile_row("edge:work", "edge-b"));

    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    open_session(&core, "s", "edge-s2", "edge:work", "active");
    let refused = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"edge:work"}),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains(vw_vocabulary::label("accessModes", "accept-edits"))
            || refused.contains(vw_vocabulary::label("accessModes", "ask")),
        "{refused}"
    );

    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"accept-edits"}),
    );
    let (edge_answer, _) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("edge-s2", "s", "edge:work", Some(9), "edge-b"),
        &[],
    )
    .await;
    let edge_text = edge_answer.to_string();
    assert!(
        edge_text.contains(vw_vocabulary::texts::EDGE_NAME),
        "{edge_answer}"
    );
    assert!(edge_text.contains(MIXED_FAILURE), "{edge_answer}");

    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    open_session(&core, "s", "edge-s3", "edge:work", "active");
    let (edge_second, _) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("edge-s3", "s", "edge:work", Some(10), "edge-b"),
        &[],
    )
    .await;
    assert!(
        edge_second.to_string().contains(ALL_CLOSED),
        "{edge_second}"
    );
    core.shutdown().await;
}

/// Safari shares the same `AgentWindows` path even when `your-safari` is off at compile time.
#[test]
fn safari_agent_windows_observer_drop_is_marked_and_cleared() {
    let mut aw = super::agent_windows::AgentWindows::default();
    aw.observe(&profile_row("safari:work", "saf-a"));
    assert!(aw.add("saf", "safari:work", 7));
    aw.observe(&profile_row("safari:work", "saf-b"));
    assert!(aw.has_dropped("saf", "safari:work"));
    assert_eq!(
        aw.nominate("saf", "safari:work", Some("saf-b")),
        (vec![], true)
    );
    assert!(aw.clear_dropped("saf", "safari:work"));
    assert!(!aw.has_dropped("saf", "safari:work"));
}

/// Ask approval card flows through `agent_close::prepare` and preserves/clears the observer-drop mark on answer.
#[tokio::test]
async fn ask_approval_card_reports_and_clears_observer_mark() {
    let core = super::read_consent_tests::core("dbk17t-ask-card").await;
    let chrome = &core.shared.chrome_control;
    super::profiles::paired(chrome, "chrome:p", None);
    observed_by_owner(&core, &profile_row("chrome:p", "p-a"));
    assert!(profile_bindings::remember(chrome, "s", "chrome:p", Some("t")).is_ok());
    created_by(&core, "s", "t", 7);
    observed_by_owner(&core, &profile_row("chrome:p", "p-b"));

    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live_status("t", "s", "chrome:p", Some(9), "p-b"),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        super::agent_close::prepare(
            &shared,
            "s",
            "s",
            "ask-close-mark",
            json!({"action":"close_browser","profileId":"chrome:p"}),
        )
        .await
    });
    let id = super::read_consent_tests::wait_for_card(&core.shared, &task).await;
    tool_calls::answer_by(
        &core.shared,
        Origin::Ui,
        "s",
        &id,
        &json!({"decision":{"type":"approve"}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
    let answer = task.await.unwrap().unwrap().await.unwrap();
    assert!(answer.to_string().contains(MIXED_FAILURE), "{answer}");
    assert!(
        !chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("s", "chrome:p")
    );
    core.shutdown().await;
}
