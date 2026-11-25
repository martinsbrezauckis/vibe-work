//! Regressions and reviewer-probe tests for DBK17U-01 sequence A (truthful later close answers)
//! and DBK17V-01 (exact helper profile isolation and ambiguous-profile refusal in `close_browser`).
//! Depends on `agent_close_incarnation_tests`, `agent_close_window_tests`, `agent_firefox_close_tests::fixture`,
//! `edge_agent_tests::fixture`, `gateway`, `profile_bindings`, and `test_support`.
use super::{
    agent_close_incarnation_tests::{
        ALL_CLOSED, close_chat_with, close_with, live_status, open_session,
    },
    agent_close_window_tests::{
        MIXED_FAILURE, REPORTED_EARLIER, closed, closes, created_by, created_in, observed_by_owner,
        profile_row, stopped, two_chats,
    },
    agent_firefox_close_tests::fixture,
    gateway, profile_bindings, test_support,
};
use crate::{Origin, ask::tool_calls};
use serde_json::json;
use vw_platform::sync::LockRecover;

#[tokio::test]
async fn dbk17v_distinct_exact_profile_request_must_not_close_other_bound_profile() {
    let (core, _) = fixture("dbk17v-wrong-exact-profile").await;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"inc-a","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"inc-b","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(&core.shared.chrome_control, upper, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "only-upper", upper, "active");
    profile_bindings::remember(&core.shared.chrome_control, "s", upper, Some("only-upper"))
        .unwrap();
    created_by(&core, "s", "only-upper", 7);
    let live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"only-upper","clientSessionId":"s","profileId":upper,"state":"active","windowId":9}],
        "profiles":both["profiles"]
    }});
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live.clone(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let outcome = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":lower}),
    )
    .await;
    let sent = calls.lock_or_recover().clone();
    assert!(
        closes(&sent).is_empty(),
        "lower request closed an upper profile window: {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|(tool, args)| tool == "chrome_session" && args["action"] == "stop"),
        "lower request stopped an upper profile session: {sent:?}"
    );
    let err = outcome.unwrap_err();
    assert!(
        err.contains(vw_vocabulary::texts::CHROME_NAME)
            && err.contains(&format!("profileId {lower}")),
        "{err}"
    );
    assert_eq!(
        core.shared
            .chrome_control
            .work
            .lock_or_recover()
            .state_of("only-upper", "s"),
        Some("active")
    );
    assert_eq!(
        profile_bindings::selected(&core.shared.chrome_control, "s").as_deref(),
        Some(upper)
    );
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        vec![7]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn dbk17v_same_exact_profile_still_closes_when_both_exact_ids_exist() {
    let (core, _) = fixture("dbk17v-same-exact-profile").await;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"same-inc","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"same-inc","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(&core.shared.chrome_control, upper, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "only-upper", upper, "active");
    profile_bindings::remember(&core.shared.chrome_control, "s", upper, Some("only-upper"))
        .unwrap();
    created_by(&core, "s", "only-upper", 7);

    let same_live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"only-upper","clientSessionId":"s","profileId":upper,"state":"active","windowId":9}],
        "profiles":both["profiles"]
    }});
    let (answer, sent_upper) =
        close_chat_with(&core, "s", upper, same_live, &[(71, 7, true)]).await;
    assert_eq!(closes(&sent_upper), vec![json!(9), json!(7)]);
    assert!(answer.to_string().contains(ALL_CLOSED), "{answer}");
    assert!(!answer.to_string().contains(REPORTED_EARLIER), "{answer}");
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        Vec::<u64>::new()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn dbk17v_ambiguous_profile_request_refuses_without_closing_or_stopping() {
    let (core, _) = fixture("dbk17v-ambiguous-profile").await;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let mixed = "chrome:AAAAaaaa-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"inc-a","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":mixed,"connected":true,"browserSessionId":"inc-b","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(&core.shared.chrome_control, upper, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "only-upper", upper, "active");
    profile_bindings::remember(&core.shared.chrome_control, "s", upper, Some("only-upper"))
        .unwrap();
    created_by(&core, "s", "only-upper", 7);

    let live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"only-upper","clientSessionId":"s","profileId":upper,"state":"active","windowId":9}],
        "profiles":both["profiles"]
    }});
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live.clone(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let outcome = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":lower}),
    )
    .await;
    let sent = calls.lock_or_recover().clone();
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        !sent
            .iter()
            .any(|(tool, args)| tool == "chrome_session" && args["action"] == "stop"),
        "{sent:?}"
    );
    let err = outcome.unwrap_err();
    assert!(
        err.contains(vw_vocabulary::texts::CHROME_NAME)
            && err.contains("2 connected profiles")
            && err.contains("Use chrome_status and supply the exact profileId for the intended profile; do not guess a profile."),
        "{err}"
    );
    assert_eq!(
        core.shared
            .chrome_control
            .work
            .lock_or_recover()
            .state_of("only-upper", "s"),
        Some("active")
    );
    assert_eq!(
        super::agent_close::remembered(&core.shared, "s", upper),
        vec![7]
    );

    assert_owner_and_edge_ambiguous_refusal(&core, lower).await;
    core.shutdown().await;
}

async fn assert_owner_and_edge_ambiguous_refusal(core: &crate::Core, lower: &str) {
    // Also refuses via connected_profiles when chrome_status omits profiles, and for Edge.
    let calls2 = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let err2 = match gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":lower}),
    )
    .await
    {
        Ok(value) => panic!("expected owner ambiguity refusal, got {value}"),
        Err(err) => err,
    };
    assert!(
        err2.contains("Use chrome_status and supply the exact profileId"),
        "{err2}"
    );
    assert_eq!(
        closes(&calls2.lock_or_recover()),
        [] as [serde_json::Value; 0]
    );

    let edge_upper = "edge:BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    let edge_mixed = "edge:BBBBbbbb-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    let edge_lower = "edge:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let edge_both = json!({"profiles":[
        {"profileId":edge_upper,"connected":true,"browserSessionId":"e-a"},
        {"profileId":edge_mixed,"connected":true,"browserSessionId":"e-b"}
    ]});
    observed_by_owner(core, &edge_both);
    let edge_err = match gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":edge_lower}),
    )
    .await
    {
        Ok(value) => panic!("expected Edge ambiguity refusal, got {value}"),
        Err(err) => err,
    };
    assert!(
        edge_err.contains(vw_vocabulary::texts::EDGE_NAME)
            && edge_err.contains("2 connected profiles"),
        "{edge_err}"
    );
}

#[tokio::test]
async fn dbk17v_live_status_ignores_mismatched_session_profile_window() {
    let (core, _) = fixture("dbk17v-live-status-profile").await;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"inc-a","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"inc-b","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(&core.shared.chrome_control, lower, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "sess-lower", lower, "active");
    profile_bindings::remember(&core.shared.chrome_control, "s", lower, Some("sess-lower"))
        .unwrap();
    let mismatched_live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[{"sessionId":"sess-lower","clientSessionId":"s","profileId":upper,"state":"active","windowId":9}],
        "profiles":both["profiles"]
    }});
    let (_answer, sent) = close_chat_with(&core, "s", lower, mismatched_live, &[]).await;
    assert!(
        closes(&sent).is_empty(),
        "mismatched session profileId in status must not nominate window 9: {sent:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn dbk17v_edge_ask_and_second_chat_isolation() {
    let core = super::read_consent_tests::core("dbk17v-edge-ask-two-chats").await;
    let chrome = &core.shared.chrome_control;
    chrome
        .set_browser_enabled(super::agent_browser::Browser::Edge, true)
        .unwrap();
    let upper = "edge:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "edge:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"inc-a","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"inc-b","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(chrome, upper, None);
    super::profiles::paired(chrome, lower, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "edge-upper", upper, "active");
    profile_bindings::remember(chrome, "s", upper, Some("edge-upper")).unwrap();
    created_by(&core, "s", "edge-upper", 7);

    test_support::chat(&core.shared, "other");
    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "other",
        &json!({"approvalPolicy":"auto-approve"}),
    );
    open_session(&core, "other", "edge-other-lower", lower, "active");
    profile_bindings::remember(chrome, "other", lower, Some("edge-other-lower")).unwrap();
    created_by(&core, "other", "edge-other-lower", 8);

    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "s",
        &json!({"approvalPolicy":"ask"}),
    );
    let live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[
            {"sessionId":"edge-upper","clientSessionId":"s","profileId":upper,"state":"active","windowId":9},
            {"sessionId":"edge-other-lower","clientSessionId":"other","profileId":lower,"state":"active","windowId":10}
        ],
        "profiles":both["profiles"]
    }});
    let live_reply = live.clone();
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live_reply.clone(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        super::agent_close::prepare(
            &shared,
            "s",
            "s",
            "ask-wrong-edge",
            json!({"action":"close_browser","profileId":lower}),
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
    let err = task.await.unwrap().unwrap().await.unwrap_err();
    assert!(
        err.contains(vw_vocabulary::texts::EDGE_NAME)
            && err.contains(&format!("profileId {lower}")),
        "{err}"
    );
    let sent = calls.lock_or_recover().clone();
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        !sent
            .iter()
            .any(|(tool, args)| tool == "chrome_session" && args["action"] == "stop"),
        "{sent:?}"
    );
    assert_eq!(
        chrome.work.lock_or_recover().state_of("edge-upper", "s"),
        Some("active")
    );
    assert_eq!(
        chrome
            .work
            .lock_or_recover()
            .state_of("edge-other-lower", "other"),
        Some("active")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn dbk17v_chrome_ask_and_second_chat_isolation() {
    let core = super::read_consent_tests::core("dbk17v-chrome-ask-two-chats").await;
    let chrome = &core.shared.chrome_control;
    let upper = "chrome:AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let lower = "chrome:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let both = json!({"profiles":[
        {"profileId":upper,"connected":true,"browserSessionId":"inc-a","extensionVersion":"0.1.15","features":["window.close"]},
        {"profileId":lower,"connected":true,"browserSessionId":"inc-b","extensionVersion":"0.1.15","features":["window.close"]}
    ]});
    super::profiles::paired(chrome, upper, None);
    super::profiles::paired(chrome, lower, None);
    observed_by_owner(&core, &both);
    open_session(&core, "s", "chr-upper", upper, "active");
    profile_bindings::remember(chrome, "s", upper, Some("chr-upper")).unwrap();
    created_by(&core, "s", "chr-upper", 7);

    test_support::chat(&core.shared, "other");
    core.shared.access.observe_request(
        "session/approvalPolicy/update",
        "other",
        &json!({"approvalPolicy":"auto-approve"}),
    );
    open_session(&core, "other", "chr-other-lower", lower, "active");
    profile_bindings::remember(chrome, "other", lower, Some("chr-other-lower")).unwrap();
    created_by(&core, "other", "chr-other-lower", 8);

    let live = json!({"isError":false,"content":[{"type":"text","text":"{}"}],"structuredContent":{
        "sessions":[
            {"sessionId":"chr-upper","clientSessionId":"s","profileId":upper,"state":"active","windowId":9},
            {"sessionId":"chr-other-lower","clientSessionId":"other","profileId":lower,"state":"active","windowId":10}
        ],
        "profiles":both["profiles"]
    }});
    let calls = test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_execute" if args["action"] == "window.close" => closed(&[]),
        "chrome_session" if args["action"] == "stop" => stopped(args),
        "chrome_status" => live.clone(),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        super::agent_close::prepare(
            &shared,
            "s",
            "s",
            "ask-wrong-chrome",
            json!({"action":"close_browser","profileId":lower}),
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
    let err = task.await.unwrap().unwrap().await.unwrap_err();
    assert!(
        err.contains(vw_vocabulary::texts::CHROME_NAME)
            && err.contains(&format!("profileId {lower}")),
        "{err}"
    );
    let sent = calls.lock_or_recover().clone();
    assert!(closes(&sent).is_empty(), "{sent:?}");
    assert!(
        !sent
            .iter()
            .any(|(tool, args)| tool == "chrome_session" && args["action"] == "stop"),
        "{sent:?}"
    );
    assert_eq!(
        chrome.work.lock_or_recover().state_of("chr-upper", "s"),
        Some("active")
    );
    assert_eq!(
        chrome
            .work
            .lock_or_recover()
            .state_of("chr-other-lower", "other"),
        Some("active")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn dbk17u_second_close_must_not_claim_older_tabs_closed() {
    let (core, _) = fixture("dbk17u-second-close").await;
    two_chats(&core);
    created_in(&core, 7);
    observed_by_owner(&core, &profile_row("chrome:work", "browser-b"));
    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (first, sent_first) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", Some(9), "browser-b"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_first), vec![json!(9)]);
    assert!(first.to_string().contains(MIXED_FAILURE), "{first}");

    open_session(&core, "s", "third-chrome", "chrome:work", "active");
    let (second, sent_second) = close_with(
        &core,
        live_status("third-chrome", "s", "chrome:work", Some(10), "browser-b"),
        &[],
    )
    .await;
    let text = second.to_string();
    assert_eq!(closes(&sent_second), vec![json!(10)]);
    assert!(
        !text.contains("the agent's own tabs were closed"),
        "{second}"
    );
    assert!(text.contains(ALL_CLOSED), "{second}");
    assert!(text.contains(REPORTED_EARLIER), "{second}");
    assert!(!text.contains("the person"), "{second}");
    core.shutdown().await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "sequence of unresolvable vs resolvable warnings"
)]
async fn dbk17u_unresolvable_warnings_keep_reported_earlier_clause_across_branches() {
    let (core, _) = fixture("dbk17u-unresolvable-variants").await;
    two_chats(&core);
    // 1. Reload settled a present window: later live close and disconnected close keep REPORTED_EARLIER.
    created_in(&core, 7);
    let (reload_first, _) = close_chat_with(
        &core,
        "s",
        "chrome:work",
        live_status("old-chrome", "s", "chrome:work", Some(9), "browser-a"),
        &[(71, 7, false), (81, 8, false)],
    )
    .await;
    assert!(
        reload_first.to_string().contains(MIXED_FAILURE),
        "{reload_first}"
    );

    open_session(&core, "s", "s-live-2", "chrome:work", "active");
    let (reload_second, _) = close_chat_with(
        &core,
        "s",
        "chrome:work",
        live_status("s-live-2", "s", "chrome:work", Some(10), "browser-a"),
        &[],
    )
    .await;
    assert!(
        reload_second.to_string().contains(ALL_CLOSED),
        "{reload_second}"
    );
    assert!(
        reload_second.to_string().contains(REPORTED_EARLIER),
        "{reload_second}"
    );

    open_session(&core, "s", "s-disc", "chrome:work", "disconnected");
    let (disc_after, sent_disc) = close_chat_with(
        &core,
        "s",
        "chrome:work",
        live_status("s-disc", "s", "chrome:work", None, "browser-a"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_disc), [] as [serde_json::Value; 0]);
    assert!(
        disc_after.to_string().contains("ended by close_browser"),
        "{disc_after}"
    );
    assert!(
        disc_after.to_string().contains(REPORTED_EARLIER),
        "{disc_after}"
    );

    // Another chat on the same profile stays clean.
    let (other_reply, _) = close_chat_with(
        &core,
        "other",
        "chrome:work",
        live_status("other-chrome", "other", "chrome:work", Some(8), "browser-a"),
        &[],
    )
    .await;
    assert!(
        other_reply.to_string().contains(ALL_CLOSED),
        "{other_reply}"
    );
    assert!(
        !other_reply.to_string().contains(REPORTED_EARLIER),
        "{other_reply}"
    );

    // Same chat on Edge stays clean.
    observed_by_owner(&core, &profile_row("edge:work", "edge-a"));
    open_session(&core, "s", "s-edge", "edge:work", "active");
    profile_bindings::remember(
        &core.shared.chrome_control,
        "s",
        "edge:work",
        Some("s-edge"),
    )
    .unwrap();
    let (edge_reply, _) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("s-edge", "s", "edge:work", Some(12), "edge-a"),
        &[],
    )
    .await;
    assert!(edge_reply.to_string().contains(ALL_CLOSED), "{edge_reply}");
    assert!(
        !edge_reply.to_string().contains(REPORTED_EARLIER),
        "{edge_reply}"
    );

    // 2. Moved agent tab in person's window on Edge -> later close on Edge keeps REPORTED_EARLIER.
    open_session(&core, "s", "s-edge-moved", "edge:work", "active");
    profile_bindings::remember(
        &core.shared.chrome_control,
        "s",
        "edge:work",
        Some("s-edge-moved"),
    )
    .unwrap();
    let (moved_first, _) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("s-edge-moved", "s", "edge:work", Some(13), "edge-a"),
        &[(71, 5, true)],
    )
    .await;
    assert!(
        moved_first.to_string().contains(MIXED_FAILURE),
        "{moved_first}"
    );

    open_session(&core, "s", "s-edge-3", "edge:work", "active");
    let (moved_second, _) = close_chat_with(
        &core,
        "s",
        "edge:work",
        live_status("s-edge-3", "s", "edge:work", Some(14), "edge-a"),
        &[],
    )
    .await;
    assert!(
        moved_second.to_string().contains(ALL_CLOSED),
        "{moved_second}"
    );
    assert!(
        moved_second.to_string().contains(REPORTED_EARLIER),
        "{moved_second}"
    );

    // 3. Unconfirmed Stop -> later close (including handed window with no own window) keeps REPORTED_EARLIER.
    observed_by_owner(&core, &profile_row("chrome:stop", "stop-a"));
    open_session(&core, "other", "o-stop-1", "chrome:stop", "active");
    profile_bindings::remember(
        &core.shared.chrome_control,
        "other",
        "chrome:stop",
        Some("o-stop-1"),
    )
    .unwrap();
    test_support::answering(&core.shared, |tool, _| match tool {
        "chrome_session" => {
            json!({"structuredContent":{"status":"unknown","error":{"code":"STOP_UNCONFIRMED"}}})
        }
        "chrome_status" => live_status("o-stop-1", "other", "chrome:stop", None, "stop-a"),
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let stop_fail = gateway::call(
        &core.shared,
        "other",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:stop"}),
    )
    .await
    .unwrap();
    assert!(
        stop_fail.to_string().contains("did not confirm Stop"),
        "{stop_fail}"
    );

    open_session(&core, "other", "o-stop-2", "chrome:stop", "active");
    profile_bindings::remember(
        &core.shared.chrome_control,
        "other",
        "chrome:stop",
        Some("o-stop-2"),
    )
    .unwrap();
    let (after_stop_fail, _) = close_chat_with(
        &core,
        "other",
        "chrome:stop",
        live_status("o-stop-2", "other", "chrome:stop", None, "stop-a"),
        &[],
    )
    .await;
    assert!(
        after_stop_fail
            .to_string()
            .contains("was stopped by close_browser")
            && after_stop_fail.to_string().contains(REPORTED_EARLIER),
        "{after_stop_fail}"
    );
    core.shutdown().await;
}

#[test]
fn safari_agent_windows_warned_marks_respect_caps_seed_and_chat_delete() {
    let mut aw = super::agent_windows::AgentWindows::default();
    let safari = "safari:work";
    let seeded = [safari.to_owned()];
    aw.seed_warned([("c0", seeded.as_slice())]);
    assert!(aw.has_warned("c0", safari));
    for i in 1..256 {
        aw.mark_warned(&format!("c{i}"), safari);
    }
    assert!(aw.has_warned("c0", safari), "256 chats fit");
    aw.mark_warned("c256", safari);
    assert!(!aw.has_warned("c0", safari), "257th chat evicts oldest");
    assert!(aw.has_warned("c1", safari));
    assert!(aw.has_warned("c256", safari));

    for p in 0..17 {
        aw.mark_warned("c1", &format!("safari:p{p}"));
    }
    assert!(
        !aw.has_warned("c1", safari),
        "17th profile mark evicts earliest entries beyond MAX_PER_CHAT"
    );
    assert!(!aw.has_warned("c1", "safari:p0"));
    assert!(aw.has_warned("c1", "safari:p1"));
    assert!(aw.has_warned("c1", "safari:p16"));

    assert!(aw.remove_chat("c1"));
    assert!(!aw.has_warned("c1", "safari:p16"));
}

#[tokio::test]
async fn dbk17u_resolvable_disconnected_window_closed_later_omits_reported_earlier() {
    let (core, _) = fixture("dbk17u-resolvable-lost-retry").await;
    two_chats(&core);
    open_session(&core, "s", "old-chrome", "chrome:work", "disconnected");
    created_in(&core, 7);

    let (first, sent_first) = close_with(
        &core,
        live_status("old-chrome", "s", "chrome:work", None, "browser-a"),
        &[],
    )
    .await;
    assert_eq!(closes(&sent_first), [] as [serde_json::Value; 0]);
    assert!(first.to_string().contains("may still be open"), "{first}");

    open_session(&core, "s", "new-chrome", "chrome:work", "active");
    let (second, sent_second) = close_with(
        &core,
        live_status("new-chrome", "s", "chrome:work", Some(9), "browser-a"),
        &[(71, 7, true)],
    )
    .await;
    assert_eq!(closes(&sent_second), vec![json!(9), json!(7)]);
    assert!(second.to_string().contains(ALL_CLOSED), "{second}");
    assert!(!second.to_string().contains(REPORTED_EARLIER), "{second}");
    core.shutdown().await;
}
