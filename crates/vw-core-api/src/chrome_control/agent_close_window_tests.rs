//! `close_browser` in Chrome / Edge closes the agent's own window first (owner 2026-11-23), then stops the session.
//! The helper's `window.close` closes only this chat's exact original-window tabs; another chat is never touched.
use super::{agent_firefox_close_tests::fixture, gateway, profile_bindings, test_support};
use serde_json::{Value, json};
use vw_platform::sync::LockRecover;

/// The answer when some own tabs stay open after a window closed: it names the possible causes (a moved tab, a reconnect
/// or browser restart, a close the browser did not confirm) and asserts none of them (DBK17S-02).
pub(super) const MIXED_FAILURE: &str = "Some of the agent's own tabs could not be closed (a tab the user moved to another window, a window from before the browser reconnected or restarted, or a close the browser did not confirm); tell the user they may still be open.";

/// The app's status observer reads a helper status (`structuredContent`), as the owner loop does for every helper
/// answer: here it learns the browser incarnation of each profile row.
pub(super) fn observed(core: &crate::Core, data: &Value) {
    profile_bindings::observe(&core.shared.chrome_control, data);
}

/// The same through the owner's own observation path (`update_pairing`), which every helper answer takes.
pub(super) fn observed_by_owner(core: &crate::Core, data: &Value) {
    super::owner::update_pairing(
        &std::sync::Arc::downgrade(&core.shared),
        &json!({"isError":false,"structuredContent":data}),
    );
}

/// A helper status that names one profile row with the given browser incarnation (none when `incarnation` is empty).
pub(super) fn profile_row(profile: &str, incarnation: &str) -> Value {
    let mut row = json!({"profileId":profile,"connected":true});
    if !incarnation.is_empty() {
        row["browserSessionId"] = json!(incarnation);
    }
    json!({"profiles":[row]})
}

pub(super) fn two_chats(core: &crate::Core) {
    let chrome = &core.shared.chrome_control;
    // The first helper status the app sees names the browser incarnation `browser-a` of the profile.
    observed(core, &status()["structuredContent"]);
    assert!(profile_bindings::remember(chrome, "s", "chrome:work", Some("old-chrome")).is_ok());
    chrome
        .tasks
        .lock_or_recover()
        .insert("old-chrome".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old-chrome", "s", "active", "chrome:work", None);
    test_support::chat(&core.shared, "other");
    assert!(
        profile_bindings::remember(chrome, "other", "chrome:work", Some("other-chrome")).is_ok()
    );
    chrome
        .tasks
        .lock_or_recover()
        .insert("other-chrome".into(), "other".into());
    chrome
        .work
        .lock_or_recover()
        .open("other-chrome", "other", "active", "chrome:work", None);
}

/// The extension negotiates `window.close` as a feature: the app sends it only to a profile that lists it.
pub(super) fn status() -> Value {
    json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"old-chrome","clientSessionId":"s","profileId":"chrome:work","state":"active","windowId":7},
            {"sessionId":"other-chrome","clientSessionId":"other","profileId":"chrome:work","state":"active","windowId":8}],
        "profiles":[{"profileId":"chrome:work","connected":true,"browserSessionId":"browser-a","extensionVersion":"0.1.15","features":["window.close"]}]}})
}

/// The helper's confirmed `window.close` answer with its complete final inventory (`(tab, window, owned)` rows).
pub(super) fn closed(tabs: &[(u64, u64, bool)]) -> Value {
    let tabs: Vec<Value> = tabs
        .iter()
        .map(|(tab, window, owned)| json!({"tabId":tab,"windowId":window,"owned":owned}))
        .collect();
    json!({"structuredContent":{"status":"completed","data":{"windowClosed":true,"allOwnedTabsClosed":true,
        "inventory":{"complete":true,"tabs":tabs}}}})
}

pub(super) fn stopped(args: &Value) -> Value {
    json!({"structuredContent":{"status":"completed","state":"stopped","sessionId":args["sessionId"]}})
}

/// A status whose session row for this chat has the given own window (or none).
pub(super) fn status_with(window: Option<u64>) -> Value {
    json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"old-chrome","clientSessionId":"s","profileId":"chrome:work","state":"active","windowId":window},
            {"sessionId":"other-chrome","clientSessionId":"other","profileId":"chrome:work","state":"active","windowId":8}],
        "profiles":[{"profileId":"chrome:work","connected":true,"browserSessionId":"browser-a","extensionVersion":"0.1.15","features":["window.close"]}]}})
}

/// The agent's `tabs.create` answer passes through the gateway; the app remembers its window for `close_browser`.
pub(super) fn created_in(core: &crate::Core, window: u64) {
    created_by(core, "s", "old-chrome", window);
}

/// The `tabs.create` answer of `chat`'s agent in its session `session`.
pub(super) fn created_by(core: &crate::Core, chat: &str, session: &str, window: u64) {
    super::agent_close::remember(
        &core.shared,
        chat,
        "chrome_execute",
        &json!({"sessionId":session,"action":"tabs.create"}),
        &json!({"isError":false,"structuredContent":{"status":"completed",
            "data":{"tab":{"tabId":window * 10 + 1,"windowId":window}}}}),
    );
}

pub(super) fn closes(sent: &[(String, Value)]) -> Vec<Value> {
    sent.iter()
        .filter(|(tool, args)| tool == "chrome_execute" && args["action"] == "window.close")
        .map(|(_, args)| args["args"]["windowId"].clone())
        .collect()
}

fn position(sent: &[(String, Value)], tool: &str, action: &str, id: &str) -> Vec<usize> {
    sent.iter()
        .enumerate()
        .filter(|(_, (t, args))| t == tool && args["action"] == action && args["sessionId"] == id)
        .map(|(i, _)| i)
        .collect()
}

/// PREDICT (red before the change): `close_browser` sent only the Stop; the agent's own Chrome window stayed open.
#[tokio::test]
async fn chrome_close_closes_the_agents_own_window_before_the_stop() {
    let (core, _) = fixture("agent-chrome-close-window").await;
    two_chats(&core);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[(81, 8, false)]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    let window = position(&sent, "chrome_execute", "window.close", "old-chrome");
    let stop = position(&sent, "chrome_session", "stop", "old-chrome");
    assert_eq!(window.len(), 1, "{sent:?}");
    assert_eq!(
        sent[window[0]].1["args"]["windowId"], 7,
        "only this session's own window: {sent:?}"
    );
    assert_eq!(stop.len(), 1, "{sent:?}");
    assert!(
        window[0] < stop[0],
        "the window closes before the Stop: {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|(_, args)| args["sessionId"] == "other-chrome"),
        "another chat's session is never touched: {sent:?}"
    );
    assert!(
        closed
            .to_string()
            .contains("the agent's own tabs were closed"),
        "{closed}"
    );
    assert_eq!(
        profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}

/// A window close the helper refuses still ends with the Stop and a free chat, and the agent is told the truth.
#[tokio::test]
async fn a_refused_window_close_still_stops_frees_the_chat_and_says_so() {
    let (core, _) = fixture("agent-chrome-close-window-refused").await;
    two_chats(&core);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => json!({"isError":true,
            "structuredContent":{"error":{"code":"BROWSER_REFUSED","message":"The window could not be closed."}}}),
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"completed","state":"stopped","sessionId":args["sessionId"]}})
        }
        "chrome_status" => status(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        position(&sent, "chrome_session", "stop", "old-chrome").len(),
        1,
        "{sent:?}"
    );
    assert!(
        closed.to_string().contains("could not be closed"),
        "{closed}"
    );
    assert_eq!(
        profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}

/// A chat that works only in a window the person handed over has no own window: nothing is closed there, only the Stop.
#[tokio::test]
async fn a_handed_window_is_never_closed() {
    let (core, _) = fixture("agent-chrome-close-handed").await;
    two_chats(&core);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"completed","state":"stopped","sessionId":args["sessionId"]}})
        }
        "chrome_status" => {
            json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
            "sessions":[{"sessionId":"old-chrome","clientSessionId":"s","profileId":"chrome:work","state":"active","windowId":null}],
            "profiles":[{"profileId":"chrome:work","connected":true,"browserSessionId":"browser-a","extensionVersion":"0.1.15","features":["window.close"]}]}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert!(
        position(&sent, "chrome_execute", "window.close", "old-chrome").is_empty(),
        "no window close without an own window: {sent:?}"
    );
    assert_eq!(
        position(&sent, "chrome_session", "stop", "old-chrome").len(),
        1,
        "{sent:?}"
    );
    assert!(!closed.to_string().contains("own tabs"), "{closed}");
    assert_eq!(
        profile_bindings::selected(&core.shared.chrome_control, "s"),
        None
    );
    core.shutdown().await;
}

/// Native 11-23 (C17H6/H7): after each lost connection the next session opened a new window and `close_browser` closed
/// only the newest. PREDICT (red before the change): only window 9 was closed; 7, which the agent opened, stayed.
#[tokio::test]
async fn older_windows_holding_this_chats_tabs_close_too() {
    let (core, _) = fixture("agent-chrome-close-older").await;
    two_chats(&core);
    // The agent's tabs.create before the lost connection opened window 7.
    created_in(&core, 7);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" && args["args"]["windowId"] == 9 => {
            closed(&[(71, 7, true), (81, 8, false), (51, 5, false)])
        }
        "chrome_execute" if args["action"] == "window.close" => {
            closed(&[(81, 8, false), (51, 5, false)])
        }
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(Some(9)),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        closes(&sent),
        vec![json!(9), json!(7)],
        "the current window, then the older window with this chat's tab; never another chat's or the person's: {sent:?}"
    );
    let window = position(&sent, "chrome_execute", "window.close", "old-chrome");
    let stop = position(&sent, "chrome_session", "stop", "old-chrome");
    assert!(window[1] < stop[0], "{sent:?}");
    assert!(
        closed
            .to_string()
            .contains("the agent's own tabs were closed"),
        "{closed}"
    );
    core.shutdown().await;
}

/// A reconnected session that only read its old tab has no own window: the window its agent created is still closed.
#[tokio::test]
async fn a_window_the_agent_created_closes_when_the_session_has_none() {
    let (core, _) = fixture("agent-chrome-close-remembered").await;
    two_chats(&core);
    created_in(&core, 7);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[(81, 8, false)]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(None),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(closes(&sent), vec![json!(7)], "{sent:?}");
    assert!(
        closed
            .to_string()
            .contains("the agent's own tabs were closed"),
        "{closed}"
    );
    assert!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work").is_empty(),
        "a closed window is forgotten"
    );
    core.shutdown().await;
}

/// After an extension reload the agent's older window no longer holds tabs the helper counts as this chat's: it is not
/// asked (that close waits about 28 s natively and changes nothing), and the agent is told that tabs may stay open.
#[tokio::test]
async fn an_agent_window_left_from_before_a_reload_is_reported() {
    let (core, _) = fixture("agent-chrome-close-reloaded").await;
    two_chats(&core);
    created_in(&core, 7);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => {
            closed(&[(71, 7, false), (81, 8, false)])
        }
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(Some(9)),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(closes(&sent), vec![json!(9)], "{sent:?}");
    assert!(closed.to_string().contains(MIXED_FAILURE), "{closed}");
    core.shutdown().await;
}

/// A chat whose session already lost its connection has no live session to name its old window: the window is kept in
/// memory and the agent is told how to close it (open a new session, then `close_browser` again).
#[tokio::test]
async fn a_lost_session_keeps_its_window_for_the_next_close() {
    let (core, _) = fixture("agent-chrome-close-lost").await;
    two_chats(&core);
    core.shared.chrome_control.work.lock_or_recover().open(
        "old-chrome",
        "s",
        "disconnected",
        "chrome:work",
        None,
    );
    created_in(&core, 7);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(None),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        closed
            .to_string()
            .contains("open a new browser session and use close_browser again"),
        "{closed}"
    );
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7]
    );
    core.shutdown().await;
}

/// DBK17R-01: the person moved an agent-created tab into their own window 5 and handed it over. The inventory lists that
/// tab as this chat's, but window 5 was never opened by the agent: it is not asked, and the agent is told a tab may stay.
/// PREDICT (red on 84e4185f9): window 5 was nominated from the inventory and its moved tab closed.
#[tokio::test]
async fn a_person_window_holding_a_moved_agent_tab_is_never_closed() {
    let (core, _) = fixture("agent-chrome-close-moved").await;
    two_chats(&core);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => {
            closed(&[(71, 5, true), (51, 5, false), (81, 8, false)])
        }
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(Some(9)),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        closes(&sent),
        vec![json!(9)],
        "only the agent's own window: {sent:?}"
    );
    assert!(closed.to_string().contains(MIXED_FAILURE), "{closed}");
    core.shutdown().await;
}

/// DBK17S-02: window 9 closed and window 7 answered "outcome unknown" (no move, no reconnect). The answer must not assert
/// a moved tab or a reconnect: it lists the possible causes, among them a close the browser did not confirm.
/// PREDICT (red before the change): the parenthesis named only a moved tab and a reconnect.
#[tokio::test]
async fn an_unconfirmed_window_close_is_named_among_the_possible_causes() {
    let (core, _) = fixture("agent-chrome-close-unknown-outcome").await;
    two_chats(&core);
    created_in(&core, 7);
    test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" && args["args"]["windowId"] == 7 => {
            json!({"isError":true,"structuredContent":{"error":{"code":"OUTCOME_UNKNOWN","message":"The extension did not answer the window close."}}})
        }
        "chrome_execute" if args["action"] == "window.close" => {
            closed(&[(71, 7, true), (81, 8, false)])
        }
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_with(Some(9)),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    let text = closed.to_string();
    assert!(text.contains(MIXED_FAILURE), "{closed}");
    assert!(
        text.contains("a close the browser did not confirm"),
        "{closed}"
    );
    core.shutdown().await;
}

/// `status_with(None)` plus the row of the session `reopen` makes (the helper's feature gate reads the session's row).
fn status_reopened() -> Value {
    let mut status = status_with(None);
    if let Some(rows) = status["structuredContent"]["sessions"].as_array_mut() {
        rows.push(json!({"sessionId":"new-chrome","clientSessionId":"s","profileId":"chrome:work","state":"active","windowId":null}));
    }
    status
}

/// A new session of chat "s" after `close_browser` released the first one (the agent opens a browser session again).
fn reopen(core: &crate::Core) {
    let chrome = &core.shared.chrome_control;
    assert!(profile_bindings::remember(chrome, "s", "chrome:work", Some("new-chrome")).is_ok());
    chrome
        .tasks
        .lock_or_recover()
        .insert("new-chrome".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("new-chrome", "s", "active", "chrome:work", None);
}

/// DBK17R-02: a window whose `window.close` the helper refused is still the chat's open window. It stays remembered, so a
/// later `close_browser` (after the agent opens a session again) nominates it again; only a confirmed close forgets it.
/// PREDICT (red before the change): every tried window was forgotten, confirmed or not: `remembered` was `[]`.
#[tokio::test]
async fn an_unconfirmed_window_close_keeps_the_window_for_a_later_retry() {
    let (core, _) = fixture("agent-chrome-close-retry").await;
    two_chats(&core);
    created_in(&core, 7);
    let refuse = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let refusing = refuse.clone();
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute"
            if args["action"] == "window.close"
                && refusing.load(std::sync::atomic::Ordering::SeqCst) =>
        {
            json!({"isError":true,
                "structuredContent":{"error":{"code":"BROWSER_REFUSED","message":"The window could not be closed."}}})
        }
        "chrome_execute" if args["action"] == "window.close" => closed(&[(81, 8, false)]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => status_reopened(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let request = json!({"action":"close_browser","profileId":"chrome:work"});
    let first = gateway::call(&core.shared, "s", "chrome_session", request.clone())
        .await
        .unwrap();
    assert_eq!(closes(&calls.lock_or_recover()), vec![json!(7)]);
    assert!(first.to_string().contains("could not be closed"), "{first}");
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7],
        "an unconfirmed close keeps the window"
    );
    // The agent opens a session again; this time the helper confirms.
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    reopen(&core);
    let second = gateway::call(&core.shared, "s", "chrome_session", request)
        .await
        .unwrap();
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        closes(&sent),
        vec![json!(7), json!(7)],
        "the retry nominates the same window: {sent:?}"
    );
    assert!(
        second
            .to_string()
            .contains("the agent's own tabs were closed"),
        "{second}"
    );
    assert!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work").is_empty(),
        "a confirmed close is forgotten"
    );
    core.shutdown().await;
}

/// DBK17R-03: a deleted chat keeps no remembered windows; an ordinary Stop keeps them for the lost-session recovery.
/// PREDICT (red before the change): `end_chat(.., true)` left the chat's entry in `agent_windows`.
#[tokio::test]
async fn a_deleted_chats_remembered_windows_are_dropped_a_stopped_chats_are_kept() {
    let (core, _) = fixture("agent-chrome-close-deleted").await;
    two_chats(&core);
    created_in(&core, 7);
    created_by(&core, "other", "other-chrome", 8);
    let chrome = &core.shared.chrome_control;
    chrome.end_chat("s", false);
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        vec![7],
        "a stopped chat keeps its window"
    );
    chrome.end_chat("s", true);
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", "chrome:work"),
        Vec::<u64>::new(),
        "a deleted chat keeps no window"
    );
    assert_eq!(
        super::agent_close::remembered(&core.shared, "other", "chrome:work"),
        vec![8],
        "another chat's windows stay"
    );
    core.shutdown().await;
}

/// DBK17R-03: the remembered windows of all chats are bounded; the chat remembered first is dropped when a new chat needs
/// room. PREDICT (red before the change): the map grew with every chat and "cap0" stayed.
#[tokio::test]
async fn remembered_windows_are_capped_in_chats_and_the_oldest_chat_goes_first() {
    let (core, _) = fixture("agent-chrome-close-cap").await;
    two_chats(&core);
    let remembered = |chat: &str| super::agent_close::remembered(&core.shared, chat, "chrome:work");
    for chat in 0..256u64 {
        created_by(&core, &format!("cap{chat}"), "old-chrome", 1000 + chat);
    }
    assert_eq!(remembered("cap0"), vec![1000], "256 chats fit");
    // Another window of a chat already held takes no new room.
    created_by(&core, "cap1", "old-chrome", 2001);
    assert_eq!(remembered("cap0"), vec![1000]);
    assert_eq!(remembered("cap1"), vec![2001, 1001]);
    // A 257th chat drops the chat remembered first, and only that one.
    created_by(&core, "cap256", "old-chrome", 1256);
    assert!(remembered("cap0").is_empty(), "the oldest chat is dropped");
    assert_eq!(remembered("cap1"), vec![2001, 1001]);
    assert_eq!(remembered("cap255"), vec![1255]);
    assert_eq!(remembered("cap256"), vec![1256]);
    core.shutdown().await;
}
