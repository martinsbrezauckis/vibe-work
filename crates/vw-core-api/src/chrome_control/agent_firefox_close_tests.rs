//! Agent close of the owned Firefox window, through the same fake owner pipe as Settings.
//! The tests pin the helper request, chat ownership and truthful session state.
use super::{PairingState, gateway, profile_binding_tests, profile_bindings, test_support};
use crate::{Origin, ask::tool_calls};
use serde_json::{Value, json};
use vw_platform::sync::LockRecover;

fn request() -> Value {
    json!({"action":"close_browser","profileId":"firefox:agent"})
}

pub(super) async fn fixture(
    name: &str,
) -> (
    crate::Core,
    std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
) {
    let core = profile_binding_tests::connected(name).await;
    assert!(
        core.shared
            .chrome_control
            .set_browser_enabled(super::agent_browser::Browser::Firefox, true)
            .is_ok()
    );
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock_or_recover() = (PairingState::WindowOpen, None);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" if args["_firefoxWindow"] == "close" => {
            json!({"structuredContent":{"firefoxWindow":"Window closed"}})
        }
        "chrome_session" if args["action"] == "open" => {
            json!({"structuredContent":{"sessionId":"fresh","profileId":args["profileId"],"state":"active"}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    (core, calls)
}

#[tokio::test]
async fn close_uses_settings_request_and_ends_this_chats_firefox_session() {
    let (core, calls) = fixture("agent-firefox-close-request").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    let reply = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap();
    assert!(reply.to_string().contains("Firefox was closed"), "{reply}");
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        sent.iter()
            .filter(|(tool, args)| tool == "chrome_status"
                && *args == json!({"_firefoxWindow":"close"}))
            .count(),
        1,
        "{sent:?}"
    );
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    assert!(profile_bindings::check(chrome, "s", "edge:work").is_ok());
    let ended = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"get","sessionId":"old"}),
    )
    .await
    .unwrap_err();
    assert!(ended.contains("ended by close_browser"), "{ended}");
    core.shutdown().await;
}

#[tokio::test]
async fn close_refuses_personal_firefox_builtin_and_unknown_without_owner_request() {
    let (core, calls) = fixture("agent-firefox-close-profiles").await;
    for profile in ["firefox:personal", "builtin", "garbage", ""] {
        let reason = gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"close_browser","profileId":profile}),
        )
        .await
        .unwrap_err();
        assert!(
            reason.contains("agent's") || reason.contains("profileId"),
            "{profile}: {reason}"
        );
    }
    let reason = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser"}),
    )
    .await
    .unwrap_err();
    assert!(reason.contains("profileId"), "{reason}");
    assert!(calls.lock_or_recover().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn chrome_close_stops_only_this_chat_and_releases_it_for_edge() {
    let (core, _) = fixture("agent-chrome-close-release").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "chrome:work", Some("old-chrome")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old-chrome".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old-chrome", "s", "active", "chrome:work", None);
    test_support::chat(&core.shared, "other");
    profile_bindings::remember(chrome, "other", "chrome:work", Some("other-chrome")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("other-chrome".into(), "other".into());
    chrome
        .work
        .lock_or_recover()
        .open("other-chrome", "other", "active", "chrome:work", None);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"completed","state":"stopped","sessionId":args["sessionId"]}})
        }
        "chrome_session" if args["action"] == "open" => {
            json!({"structuredContent":{"sessionId":"new-edge","profileId":args["profileId"],"state":"active"}})
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
    assert!(closed.to_string().contains("stopped"), "{closed}");
    let sent = calls.lock_or_recover().clone();
    assert_eq!(
        sent.iter()
            .filter(|(tool, args)| tool == "chrome_session"
                && args["action"] == "stop"
                && args["sessionId"] == "old-chrome")
            .count(),
        1,
        "{sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|(_, args)| args["sessionId"] == "other-chrome")
    );
    assert_eq!(
        chrome
            .work
            .lock_or_recover()
            .state_of("other-chrome", "other"),
        Some("active")
    );
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    let ended = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"get","sessionId":"old-chrome"}),
    )
    .await
    .unwrap_err();
    assert!(ended.contains("ended by close_browser"), "{ended}");
    let edge = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"edge:work"}),
    )
    .await
    .unwrap();
    assert_eq!(
        edge["vwMcpResult"]["structuredContent"]["state"], "active",
        "{edge}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn disconnected_chrome_close_ends_without_a_reconnect_refusal_or_late_rebind() {
    let (core, _) = fixture("agent-chrome-close-disconnected").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "chrome:work", Some("lost")).unwrap();
    chrome
        .work
        .lock_or_recover()
        .open("lost", "s", "disconnected", "chrome:work", None);
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    assert!(closed.to_string().contains("ended"), "{closed}");
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"chrome:work","connected":true}],"sessions":[{"sessionId":"lost","clientSessionId":"s","profileId":"chrome:work","state":"active"}]}}),
    );
    assert_eq!(
        chrome.work.lock_or_recover().state_of("lost", "s"),
        Some("disconnected")
    );
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    assert!(profile_bindings::check(chrome, "s", "edge:work").is_ok());
    core.shutdown().await;
}

#[tokio::test]
async fn disconnected_chrome_row_remains_closable_after_its_binding_was_cleared() {
    let (core, _) = fixture("agent-chrome-close-after-binding-release").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "chrome:work", Some("lost")).unwrap();
    chrome
        .work
        .lock_or_recover()
        .open("lost", "s", "disconnected", "chrome:work", None);
    profile_bindings::close_agent(chrome, "s", "chrome:work");
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    assert!(
        answer.to_string().contains("ended by close_browser"),
        "{answer}"
    );
    let ended = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"get","sessionId":"lost"}),
    )
    .await
    .unwrap_err();
    assert!(ended.contains("ended by close_browser"), "{ended}");
    core.shutdown().await;
}

#[tokio::test]
async fn unknown_id_open_is_freed_and_a_late_active_row_cannot_rebind() {
    let (core, _) = fixture("agent-chrome-close-unknown-id").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::mark_uncertain(chrome, "s", None, Some("chrome:work"));
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "delete" => {
            json!({"structuredContent":{"status":"completed"}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"chrome:work"}),
    )
    .await
    .unwrap();
    assert!(answer.to_string().contains("did not confirm"), "{answer}");
    assert!(!profile_bindings::uncertain(chrome, "s"));
    assert!(
        calls
            .lock_or_recover()
            .iter()
            .any(|(tool, args)| tool == "chrome_session"
                && args["action"] == "delete"
                && args["_clientSessionId"] == "s"
                && args["profileId"] == "chrome:work")
    );
    let status = json!({"version":1,"clientId":"vibework-owner","profiles":[{"profileId":"chrome:work","connected":true}],"sessions":[{"sessionId":"late","clientSessionId":"s","profileId":"chrome:work","state":"active"}]});
    profile_bindings::migrate(chrome, "s", &status).unwrap();
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    assert!(profile_bindings::check(chrome, "s", "edge:work").is_ok());
    core.shutdown().await;
}

#[tokio::test]
async fn edge_close_releases_even_when_stop_is_unconfirmed() {
    let (core, _) = fixture("agent-edge-close-unconfirmed").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "edge:work", Some("edge-old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("edge-old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("edge-old", "s", "active", "edge:work", None);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"unknown","error":{"code":"STOP_UNCONFIRMED","message":"Cleanup was not confirmed."}}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let closed = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"close_browser","profileId":"edge:work"}),
    )
    .await
    .unwrap();
    assert!(closed.to_string().contains("did not confirm"), "{closed}");
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_session"
                && args["action"] == "stop"
                && args["sessionId"] == "edge-old")
            .count(),
        1
    );
    assert_eq!(profile_bindings::selected(chrome, "s"), None);
    assert!(profile_bindings::check(chrome, "s", "chrome:work").is_ok());
    core.shutdown().await;
}

#[tokio::test]
async fn close_refuses_another_chats_live_firefox() {
    let (core, calls) = fixture("agent-firefox-close-other-chat").await;
    test_support::chat(&core.shared, "other");
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "other", "firefox:agent", Some("theirs")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("theirs".into(), "other".into());
    chrome
        .work
        .lock_or_recover()
        .open("theirs", "other", "active", "firefox:agent", None);
    let reason = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap_err();
    assert!(reason.contains("another chat"), "{reason}");
    assert!(calls.lock_or_recover().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn close_refuses_another_chats_uncertain_firefox() {
    let (core, calls) = fixture("agent-firefox-close-uncertain-other").await;
    test_support::chat(&core.shared, "other");
    profile_bindings::mark_uncertain(
        &core.shared.chrome_control,
        "other",
        Some("pending"),
        Some("firefox:agent"),
    );
    let reason = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap_err();
    assert!(reason.contains("another chat"), "{reason}");
    assert!(calls.lock_or_recover().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn close_of_already_closed_firefox_is_plain_success_without_owner_request() {
    let (core, calls) = fixture("agent-firefox-already-closed").await;
    *core
        .shared
        .chrome_control
        .owner
        .firefox_pairing
        .lock_or_recover() = (PairingState::WindowClosed, None);
    let reply = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap();
    assert!(reply.to_string().contains("already closed"), "{reply}");
    assert!(calls.lock_or_recover().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn failed_helper_close_preserves_the_firefox_binding() {
    let (core, calls) = fixture("agent-firefox-close-failed").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    let calls = {
        let calls = calls.clone();
        let replacement = test_support::answering(&core.shared, |tool, args| match tool {
            "chrome_status" if args["_firefoxWindow"] == "close" => {
                json!({"structuredContent":{"error":{"code":"WINDOW_CLOSE_FAILED","message":"The owned Firefox window could not close."}}})
            }
            _ => json!({"structuredContent":{"status":"completed"}}),
        });
        assert!(calls.lock_or_recover().is_empty());
        replacement
    };
    let reason = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap_err();
    assert!(reason.contains("could not close"), "{reason}");
    assert_eq!(
        profile_bindings::selected(chrome, "s").as_deref(),
        Some("firefox:agent")
    );
    assert_eq!(
        chrome.work.lock_or_recover().state_of("old", "s"),
        Some("active")
    );
    assert_eq!(
        chrome.owner.firefox_pairing.lock_or_recover().0,
        PairingState::WindowOpen
    );
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_status" && args["_firefoxWindow"] == "close")
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn settings_close_still_uses_the_same_owner_request() {
    let (core, calls) = fixture("settings-firefox-close-path").await;
    core.chrome_control(
        crate::Origin::Ui,
        super::ChromeControlRequest::CloseFirefox {},
    )
    .await
    .unwrap();
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_status"
                && *args == json!({"_firefoxWindow":"close"}))
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn ask_needs_a_close_approval_while_accept_edits_and_full_access_run() {
    for (mode, should_close) in [
        ("ask", false),
        ("accept-edits", true),
        ("auto-approve", true),
    ] {
        let (core, calls) = fixture(&format!("firefox-close-mode-{mode}")).await;
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let result = gateway::call(&core.shared, "s", "chrome_session", request()).await;
        assert_eq!(result.is_ok(), should_close, "{mode}: {result:?}");
        assert_eq!(
            calls
                .lock_or_recover()
                .iter()
                .filter(|(tool, args)| tool == "chrome_status" && args["_firefoxWindow"] == "close")
                .count(),
            usize::from(should_close)
        );
        core.shutdown().await;
    }
}

#[tokio::test]
async fn chrome_and_edge_close_obey_ask_accept_edits_and_full_access() {
    for profile in ["chrome:work", "edge:work"] {
        for (mode, should_stop) in [
            ("ask", false),
            ("accept-edits", true),
            ("auto-approve", true),
        ] {
            let (core, calls) =
                fixture(&format!("close-mode-{}-{mode}", profile.replace(':', "-"))).await;
            let chrome = &core.shared.chrome_control;
            profile_bindings::remember(chrome, "s", profile, Some("old")).unwrap();
            chrome
                .tasks
                .lock_or_recover()
                .insert("old".into(), "s".into());
            chrome
                .work
                .lock_or_recover()
                .open("old", "s", "active", profile, None);
            core.shared.access.observe_request(
                "session/agent/update",
                "s",
                &json!({"agentName":mode}),
            );
            let outcome = gateway::call(
                &core.shared,
                "s",
                "chrome_session",
                json!({"action":"close_browser","profileId":profile}),
            )
            .await;
            assert_eq!(
                outcome.is_ok(),
                should_stop,
                "{profile} {mode}: {outcome:?}"
            );
            assert_eq!(
                calls
                    .lock_or_recover()
                    .iter()
                    .filter(|(tool, args)| tool == "chrome_session" && args["action"] == "stop")
                    .count(),
                usize::from(should_stop)
            );
            core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn ask_card_approves_the_exact_close_before_any_stop_is_sent() {
    let core = super::read_consent_tests::core("agent-close-ask-card").await;
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"completed","state":"stopped"}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        let mut args = json!({"action":"close_browser","profileId":"chrome:p"});
        crate::agent_tools::prepare(&shared, "s", "s", "ask-close", "chrome_session", &mut args)
            .await
    });
    let id = super::read_consent_tests::wait_for_card(&core.shared, &task).await;
    assert!(calls.lock_or_recover().is_empty());
    tool_calls::answer_by(
        &core.shared,
        Origin::Ui,
        "s",
        &id,
        &json!({"decision":{"type":"approve"}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
    let execution = task.await.unwrap().unwrap().unwrap();
    let reply = execution.await.unwrap();
    assert!(reply.to_string().contains("stopped"), "{reply}");
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_session" && args["action"] == "stop")
            .count(),
        1
    );
    core.shutdown().await;
}

/// In Ask mode a subagent (source != owner chat) is refused before any card, like its `stop`.
#[tokio::test]
async fn ask_mode_subagent_cannot_request_close_of_its_parents_browser() {
    let core = super::read_consent_tests::core("agent-close-ask-subagent").await;
    let calls = test_support::answering(
        &core.shared,
        |_, _| json!({"structuredContent":{"status":"completed","state":"stopped"}}),
    );
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"child","parentSessionId":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    for profile in ["chrome:p", "firefox:agent"] {
        let mut args = json!({"action":"close_browser","profileId":profile});
        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::agent_tools::prepare(
                &core.shared,
                "s",
                "child",
                "ask-close-child",
                "chrome_session",
                &mut args,
            ),
        )
        .await
        .expect("a subagent's close_browser must be refused at once, not wait for a card");
        let reason = answer.err().unwrap_or_default();
        assert!(
            reason.contains("subagents cannot use"),
            "{profile}: {reason}"
        );
        assert!(calls.lock_or_recover().is_empty(), "{profile}");
        assert!(
            core.shared
                .store
                .view("s")
                .is_none_or(|view| !view.state.to_string().contains("ask-close-child")),
            "{profile}: no approval card for the subagent's request"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn cancelled_turn_cannot_close_the_agent_firefox() {
    let (core, calls) = fixture("agent-firefox-close-cancelled").await;
    crate::stops::stopped(&core.shared, "s");
    let before = calls.lock_or_recover().len();
    let reason = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap_err();
    assert!(
        reason.contains("Stop") || reason.contains("stopped"),
        "{reason}"
    );
    assert!(
        !calls.lock_or_recover()[before..]
            .iter()
            .any(|(tool, args)| tool == "chrome_status" && args["_firefoxWindow"] == "close")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn unanswered_firefox_new_window_preserves_helper_text_and_adds_recovery_hint() {
    let (core, _) = fixture("agent-firefox-new-window-hint").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    let calls = test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_execute" if args["action"] == "tabs.create" => {
            json!({"structuredContent":{"status":"unknown","error":{"code":"OUTCOME_UNKNOWN","message":"Firefox did not answer the new-window request. Inspect its windows before retrying."}}})
        }
        "chrome_status" if args["_firefoxWindow"] == "close" => {
            json!({"structuredContent":{"firefoxWindow":"Window closed"}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let unknown = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"old","action":"tabs.create","args":{"url":"https://example.com"}}),
    )
    .await
    .unwrap();
    assert!(
        unknown.to_string().contains(
            "Firefox did not answer the new-window request. Inspect its windows before retrying."
        ),
        "{unknown}"
    );
    assert!(
        unknown
            .to_string()
            .contains("chrome_session close_browser (profileId firefox:agent)"),
        "{unknown}"
    );
    let closed = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap();
    assert!(closed.to_string().contains("was closed"), "{closed}");
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_status" && args["_firefoxWindow"] == "close")
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn unconfirmed_firefox_stop_adds_the_close_recovery_hint() {
    let (core, _) = fixture("agent-firefox-stop-hint").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_session" if args["action"] == "stop" => {
            json!({"structuredContent":{"status":"failed","error":{"code":"STOP_UNCONFIRMED","message":"Ask the user to close this chat's agent window by hand."}}})
        }
        _ => json!({"structuredContent":{"status":"completed"}}),
    });
    let stop = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"stop","sessionId":"old"}),
    )
    .await
    .unwrap();
    assert!(
        stop.to_string()
            .contains("Ask the user to close this chat's agent window by hand."),
        "{stop}"
    );
    assert!(
        stop.to_string()
            .contains("chrome_session close_browser (profileId firefox:agent)"),
        "{stop}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn firefox_close_after_stop_unconfirmed_ends_the_old_session_and_frees_edge() {
    let (core, calls) = fixture("agent-firefox-close-after-stop-unconfirmed").await;
    let chrome = &core.shared.chrome_control;
    profile_bindings::remember(chrome, "s", "firefox:agent", Some("old")).unwrap();
    chrome
        .tasks
        .lock_or_recover()
        .insert("old".into(), "s".into());
    chrome
        .work
        .lock_or_recover()
        .open("old", "s", "active", "firefox:agent", None);
    super::owner::update_work_for_test(
        &core.shared,
        &json!({"structuredContent":{"version":1,"clientId":"vibework-owner","profiles":[],"sessions":[{"sessionId":"old","clientSessionId":"s","profileId":"firefox:agent","state":"disconnected","desiredState":"stopped","capture":"unknown","suspensionReason":"STOP_UNCONFIRMED"}]}}),
    );
    let answer = gateway::call(&core.shared, "s", "chrome_session", request())
        .await
        .unwrap();
    assert!(answer.to_string().contains("was closed"), "{answer}");
    let ended = gateway::call(
        &core.shared,
        "s",
        "chrome_session",
        json!({"action":"get","sessionId":"old"}),
    )
    .await
    .unwrap_err();
    assert!(ended.contains("ended by close_browser"), "{ended}");
    assert!(profile_bindings::check(chrome, "s", "edge:work").is_ok());
    assert_eq!(
        calls
            .lock_or_recover()
            .iter()
            .filter(|(tool, args)| tool == "chrome_status" && args["_firefoxWindow"] == "close")
            .count(),
        1
    );
    core.shutdown().await;
}
