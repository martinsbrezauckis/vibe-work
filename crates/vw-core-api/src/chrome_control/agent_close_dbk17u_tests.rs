//! Regression and reviewer-probe tests for DBK17U-01 sequence B (uncertain delete keeps dropped-window warning)
//! and DBK17U-02 (exact helper profile matching in `AgentWindows` with live-status profile resolution).
//! Depends on `agent_close_incarnation_tests`, `agent_close_window_tests`, `agent_firefox_close_tests::fixture`,
//! `edge_agent_tests::fixture`, `gateway`, `profile_bindings`, and `test_support`.
use super::{
    agent_close_incarnation_tests::{
        ALL_CLOSED, close_chat_with, close_with, live_status, open_session,
    },
    agent_close_window_tests::{
        MIXED_FAILURE, closed, closes, created_by, created_in, observed_by_owner, profile_row,
        stopped, two_chats,
    },
    agent_firefox_close_tests::fixture,
    edge_agent_tests, gateway, profile_bindings, test_support,
};
use crate::{Origin, ask::tool_calls};
use serde_json::json;
use vw_platform::sync::LockRecover;

#[tokio::test]
async fn dbk17u_uncertain_delete_must_not_erase_older_window_warning() {
    let (core, _) = fixture("dbk17u-uncertain-drop").await;
    two_chats(&core);
    created_in(&core, 7);
    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));
    let chrome = &core.shared.chrome_control;
    profile_bindings::mark_uncertain(chrome, "s", None, Some("chrome:work"));
    let (first, sent) = close_with(
        &core,
        live_status("old-chrome", "s", "chrome:work", None, "browser-b"),
        &[],
    )
    .await;
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(first.to_string().contains("may still be open"), "{first}");
    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (second, sent_second) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", Some(9), "browser-b"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_second), vec![json!(9)]);
    assert!(!second.to_string().contains(ALL_CLOSED), "{second}");
    core.shutdown().await;
}

#[test]
fn dbk17u_distinct_helper_profile_ids_must_not_share_window_candidates() {
    let mut aw = super::agent_windows::AgentWindows::default();
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    aw.observe(&profile_row(upper, "same-incarnation"));
    assert!(aw.add("one-chat", upper, 7));
    assert_eq!(
        aw.nominate("one-chat", lower, Some("same-incarnation")),
        (vec![], false)
    );
}

#[tokio::test]
async fn a_same_profile_window_opened_and_closed_under_one_spelling_still_closes() {
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let core = edge_agent_tests::fixture(
        "dbk17u-same-spelling",
        json!([{"profileId":upper,"name":"Work","connected":true}]),
    )
    .await;
    observed_by_owner(&core, &profile_row(upper, "inc-a"));
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => live_status("s1", "s", upper, Some(9), "inc-a"),
        "chrome_session" if args["action"] == "open" => {
            json!({"structuredContent":{"sessionId":"s1","profileId":upper,"state":"active"}})
        }
        "chrome_execute" if args["action"] == "tabs.create" => {
            json!({"isError":false,"structuredContent":{"status":"completed","data":{"tab":{"tabId":71,"windowId":7}}}})
        }
        "chrome_execute" if args["action"] == "window.close" && args["args"]["windowId"] == 9 => {
            closed(&[(71, 7, true), (81, 8, false)])
        }
        "chrome_execute" if args["action"] == "window.close" => closed(&[(81, 8, false)]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":upper}),
    )
    .await
    .unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"s1","action":"tabs.create","args":{"url":"https://example.test/"}}),
    )
    .await
    .unwrap();
    let (answer, sent) = close_chat_with(
        &core,
        "s",
        upper,
        live_status("s1", "s", upper, Some(9), "inc-a"),
        &[(71, 7, true), (81, 8, false)],
    )
    .await;
    assert_eq!(closes(&sent), vec![json!(9), json!(7)]);
    assert!(answer.to_string().contains(ALL_CLOSED), "{answer}");
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        Vec::<u64>::new()
    );
    core.shutdown().await;
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one scenario read top to bottom")]
async fn canonical_selected_profile_still_closes_uppercase_chrome_profiles_own_window() {
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let core = edge_agent_tests::fixture(
        "dbk17u-canonical-selected-close",
        json!([{"profileId":upper,"name":"Work","connected":true}]),
    )
    .await;
    observed_by_owner(&core, &profile_row(upper, "inc-a"));
    let next_open = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let opens = next_open.clone();
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => live_status("s1", "s", upper, Some(9), "inc-a"),
        "chrome_session" if args["action"] == "open" => {
            let n = opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            json!({"structuredContent":{"sessionId":format!("s{n}"),"profileId":upper,"state":"active"}})
        }
        "chrome_execute" if args["action"] == "tabs.create" => {
            json!({"isError":false,"structuredContent":{"status":"completed","data":{"tab":{"tabId":71,"windowId":7}}}})
        }
        "chrome_execute" if args["action"] == "window.close" && args["args"]["windowId"] == 9 => {
            closed(&[(71, 7, true), (81, 8, false)])
        }
        "chrome_execute" if args["action"] == "window.close" => closed(&[(81, 8, false)]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let status = gateway::call(&core.shared, "s", "chrome_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        status["vwMcpResult"]["structuredContent"]["profileChoices"][0]["profileId"],
        lower
    );
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":lower}),
    )
    .await
    .unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"s1","action":"tabs.create","args":{"url":"https://example.test/"}}),
    )
    .await
    .unwrap();
    // Agent calls close_browser using the canonical lowercase profileId from chrome_status.
    let (answer, sent) = close_chat_with(
        &core,
        "s",
        lower,
        live_status("s1", "s", upper, Some(9), "inc-a"),
        &[(71, 7, true), (81, 8, false)],
    )
    .await;
    assert_eq!(closes(&sent), vec![json!(9), json!(7)], "{sent:?}");
    assert!(answer.to_string().contains(ALL_CLOSED), "{answer}");
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        Vec::<u64>::new()
    );

    // Disconnected uppercase session closed via canonical lowercase profileId still warns about unclosed window.
    let opens2 = next_open.clone();
    test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => live_status("s2", "s", upper, Some(9), "inc-a"),
        "chrome_session" if args["action"] == "open" => {
            let n = opens2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            json!({"structuredContent":{"sessionId":format!("s{n}"),"profileId":upper,"state":"active"}})
        }
        "chrome_execute" if args["action"] == "tabs.create" => {
            json!({"isError":false,"structuredContent":{"status":"completed","data":{"tab":{"tabId":72,"windowId":7}}}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":lower}),
    )
    .await
    .unwrap();
    gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"s2","action":"tabs.create","args":{"url":"https://example.test/"}}),
    )
    .await
    .unwrap();
    open_session(&core, "s", "s2", upper, "disconnected");
    core.shared.chrome_control.owner.connected_profiles.clear();
    let (disconnected, sent_disconnected) = close_chat_with(
        &core,
        "s",
        lower,
        live_status("s2", "s", upper, None, "inc-a"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_disconnected), Vec::<serde_json::Value>::new());
    assert!(
        disconnected
            .to_string()
            .contains("open a new browser session and use close_browser again"),
        "{disconnected}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn two_distinct_helper_profiles_with_same_canonical_id_stay_isolated_in_close_browser() {
    let (core, _) = fixture("dbk17u-two-exact-profiles").await;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both_profiles = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"same-inc","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"same-inc","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(&core.shared.chrome_control, upper, None);
    observed_by_owner(&core, &both_profiles);
    open_session(&core, "s", "sess-upper", upper, "active");
    created_by(&core, "s", "sess-upper", 7);
    open_session(&core, "s", "sess-lower", lower, "active");
    profile_bindings::remember(&core.shared.chrome_control, "s", lower, Some("sess-lower"))
        .unwrap();
    let live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"sess-lower","clientSessionId":"s","profileId":lower,"state":"active","windowId":9}],
        "profiles":both_profiles["profiles"]
    }});
    let (answer, sent) =
        close_chat_with(&core, "s", lower, live, &[(71, 7, false), (81, 8, false)]).await;
    assert_eq!(closes(&sent), vec![json!(9)], "{sent:?}");
    assert!(answer.to_string().contains(ALL_CLOSED), "{answer}");
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        vec![7]
    );
    core.shutdown().await;
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one scenario read top to bottom")]
async fn uncertain_delete_preserves_dropped_mark_across_edge_ask_and_second_chat() {
    let core = super::read_consent_tests::core("dbk17u-uncertain-variants").await;
    let chrome = &core.shared.chrome_control;
    super::profiles::paired(chrome, "edge:work", None);
    super::profiles::paired(chrome, "chrome:work", None);
    observed_by_owner(
        &core,
        &json!({"profiles":[
            {"profileId":"edge:work","connected":true,"browserSessionId":"edge-a"},
            {"profileId":"chrome:work","connected":true,"browserSessionId":"chrome-a"}
        ]}),
    );
    profile_bindings::remember(chrome, "s", "edge:work", Some("edge-s1")).unwrap();
    open_session(&core, "s", "edge-s1", "edge:work", "disconnected");
    created_by(&core, "s", "edge-s1", 7);
    test_support::chat(&core.shared, "other");
    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "other",
        &json!({"approvalPolicy":"auto-approve"}),
    );
    profile_bindings::remember(chrome, "other", "chrome:work", Some("other-chrome")).unwrap();
    open_session(&core, "other", "other-chrome", "chrome:work", "active");
    created_by(&core, "other", "other-chrome", 8);
    observed_by_owner(
        &core,
        &json!({"profiles":[
            {"profileId":"edge:work","connected":true,"browserSessionId":"edge-b"},
            {"profileId":"chrome:work","connected":true,"browserSessionId":"chrome-b"}
        ]}),
    );
    profile_bindings::mark_uncertain(chrome, "s", None, Some("edge:work"));

    // Ask mode approval card runs the uncertain delete; the mark must stay.
    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "s",
        &json!({"approvalPolicy":"ask"}),
    );
    test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live_status("edge-s2", "s", "edge:work", Some(9), "edge-b"),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        super::agent_close::prepare(
            &shared,
            "s",
            "s",
            "ask-uncertain-del",
            json!({"action":"close_browser","profileId":"edge:work"}),
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
    let first = task.await.unwrap().unwrap().await.unwrap();
    assert!(first.to_string().contains("may still be open"), "{first}");
    assert!(
        chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("s", "edge:work")
    );
    assert!(
        chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("other", "chrome:work")
    );

    // Reopen in Full access: live close warns and clears the mark; other chat stays untouched.
    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "s",
        &json!({"approvalPolicy":"auto-approve"}),
    );
    open_session(&core, "s", "edge-s2", "edge:work", "active");
    let (second, sent) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("edge-s2", "s", "edge:work", Some(9), "edge-b"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent), vec![json!(9)]);
    assert!(second.to_string().contains(MIXED_FAILURE), "{second}");
    assert!(!second.to_string().contains(ALL_CLOSED), "{second}");
    assert!(
        !chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("s", "edge:work")
    );
    assert!(
        chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("other", "chrome:work")
    );

    // An uncertain open WITH a live session ID does run close_own_windows (the gateway refuses its window.close while
    // the open is unconfirmed, so the answer warns), and that live attempt DOES clear the mark.
    open_session(&core, "other", "other-reopen", "chrome:work", "active");
    profile_bindings::mark_uncertain(chrome, "other", Some("other-reopen"), Some("chrome:work"));
    let (other_reply, _) = close_chat_with(
        &core,
        "other",
        "chrome:work",
        live_status("other-reopen", "other", "chrome:work", Some(10), "chrome-b"),
        &[],
    )
    .await;
    assert!(
        other_reply.to_string().contains("could not be closed"),
        "{other_reply}"
    );
    assert!(
        !other_reply.to_string().contains(ALL_CLOSED),
        "{other_reply}"
    );
    assert!(
        !chrome
            .agent_windows
            .lock_or_recover()
            .has_dropped("other", "chrome:work")
    );
    core.shutdown().await;
}

#[test]
fn agent_windows_exact_match_for_add_of_nominate_forget_and_canonical_for_marks() {
    let mut aw = super::agent_windows::AgentWindows::default();
    let edge_upper = "edge:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let edge_lower = "edge:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let safari_upper = "safari:BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    let safari_lower = "safari:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    aw.observe(&json!({"profiles":[
        {"profileId":edge_upper,"connected":true,"browserSessionId":"edge-1"},
        {"profileId":safari_upper,"connected":true,"browserSessionId":"saf-1"}
    ]}));
    assert!(aw.add("c", edge_upper, 7));
    assert!(!aw.add("c", edge_upper, 7), "exact duplicate returns false");
    assert!(aw.add("c", edge_lower, 7), "distinct exact ID is distinct");
    assert!(aw.add("c", safari_upper, 8));

    assert_eq!(aw.of("c", edge_upper), vec![7]);
    assert_eq!(aw.of("c", edge_lower), vec![7]);
    assert_eq!(aw.of("c", safari_lower), Vec::<u64>::new());

    assert!(aw.forget("c", edge_lower, |w| w == 7));
    assert_eq!(aw.of("c", edge_upper), vec![7]);
    assert_eq!(aw.of("c", edge_lower), Vec::<u64>::new());

    // Browser restart observed under canonical lowercase still drops edge_upper and records canonical mark.
    aw.observe(&profile_row(edge_lower, "edge-2"));
    assert_eq!(aw.of("c", edge_upper), Vec::<u64>::new());
    assert!(aw.has_dropped("c", edge_upper));
    assert!(aw.has_dropped("c", edge_lower));
    assert_eq!(aw.of("c", safari_upper), vec![8], "Safari untouched");
}
