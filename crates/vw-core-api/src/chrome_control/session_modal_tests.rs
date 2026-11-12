//! Retained browser actions recheck named app modals after resume and keep person Stop authoritative.
//! Recording-helper tests combine session handover with owned-window closure without a live browser.
use super::{gateway, output_tests, read_consent_tests, test_support};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[tokio::test]
async fn retained_window_close_rechecks_a_modal_opened_during_resume() {
    for batch in [false, true] {
        let core = read_consent_tests::core(&format!("retained-window-modal-{batch}")).await;
        let seen = test_support::answering(&core.shared, resuming_helper(&core.shared));
        let question = core.shared.asks.open(
            "user_input",
            Some("s".into()),
            vw_ask::Requester::Agent,
            vec![vw_ask::Choice::No],
            None,
        );
        super::end_turn(&core.shared, "s");
        let action = json!({"sessionId":"t","action":"window.close","args":{}});
        let held = gateway::call(&core.shared, "s", "chrome_execute", action.clone())
            .await
            .unwrap_err();
        assert_eq!(Some(held.as_str()), core.shared.asks.question_hold("s"));
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, args)| tool == "chrome_execute" || args["action"] == "resume")
        );
        core.shared.asks.clear_question_holds(Some("s"));
        let args = if batch {
            json!({"sessionId":"t","steps":[{"action":"window.close","args":{}}]})
        } else {
            action
        };
        let first = gateway::call(&core.shared, "s", "chrome_execute", args.clone()).await;
        let refusal = match first {
            Ok(value) => value.to_string(),
            Err(reason) => reason,
        };
        assert!(
            refusal.contains("APP_DIALOG_OPEN") && refusal.contains("Choose a folder"),
            "{refusal}"
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(tool, _)| tool == "chrome_execute")
        );
        core.shared.browser_dialogs.set_ui(None);
        let result = gateway::call(&core.shared, "s", "chrome_execute", args)
            .await
            .unwrap();
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{result}"
        );
        let calls = seen.lock().unwrap().clone();
        assert_eq!(
            calls
                .iter()
                .filter(|(tool, args)| tool == "chrome_session" && args["action"] == "resume")
                .count(),
            1
        );
        assert!(
            !calls
                .iter()
                .any(|(tool, args)| tool == "chrome_session" && args["action"] == "open")
        );
        let executed: Vec<_> = calls
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .collect();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].1["sessionId"], "t");
        let caps = executed[0].1["_approvedCapabilities"].as_array().unwrap();
        assert!(caps.contains(&json!("tabs.close_owned")));
        assert!(caps.contains(&json!("tabs.inspect_other")));
        assert!(!caps.contains(&json!("page.commit")));
        assert!(
            core.shared
                .asks
                .list()
                .iter()
                .all(|ask| ask.id == question.ask.id)
        );
        drop(question);
        core.shutdown().await;
    }
}

#[tokio::test]
async fn a_prepared_action_keeps_person_stop_ahead_of_an_open_app_modal() {
    let core = read_consent_tests::core("prepared-stop-app-modal").await;
    let seen =
        test_support::answering(&core.shared, |_, _| panic!("stopped action reached helper"));
    core.shared
        .browser_dialogs
        .set_ui(Some("Choose a folder".into()));
    let _ = core
        .shared
        .chrome_control
        .person_stopped(&[super::person_stop::Stopped {
            chat: "s".into(),
            browser: super::agent_browser::Browser::Chrome,
            turn: None,
            connection_ended: false,
        }]);
    let args = json!({"sessionId":"t","action":"window.close","args":{}});
    for result in [
        gateway::call(&core.shared, "s", "chrome_execute", args.clone()).await,
        gateway::call_one_inner(&core.shared, "s", "chrome_execute", args, None, None).await,
    ] {
        let reason = result.unwrap_err();
        assert!(reason.contains("user pressed Stop"), "{reason}");
        assert!(!reason.contains("retry"), "{reason}");
    }
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

fn resuming_helper(
    shared: &Arc<crate::core::Shared>,
) -> impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static {
    let weak = Arc::downgrade(shared);
    let resumed = AtomicBool::new(false);
    move |tool, args| {
        if tool == "chrome_status" {
            output_tests::mcp(
                json!({"sessions":[{"sessionId":"t","profileId":"chrome:p","state":if resumed.load(Ordering::SeqCst) {"active"} else {"idle"}}],
                    "profiles":[{"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.7","features":["window.close"],"extensionCapabilities":["tabs.list.truncated"]}]}),
            )
        } else if tool == "chrome_session" && args["action"] == "resume" {
            resumed.store(true, Ordering::SeqCst);
            weak.upgrade()
                .unwrap()
                .browser_dialogs
                .set_ui(Some("Choose a folder".into()));
            output_tests::mcp(json!({"sessionId":"t","profileId":"chrome:p","state":"active"}))
        } else {
            output_tests::mcp(
                json!({"status":"completed","data":{"windowClosed":false,"remainingTabs":[{"tabId":9,"reason":"Personal tab"}]}}),
            )
        }
    }
}
