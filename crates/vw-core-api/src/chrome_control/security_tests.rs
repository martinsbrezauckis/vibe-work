//! Execution-time checks for sensitive Chrome steps through the real gateway and private owner fixture.
//! The helper/extension tests cover native tab and download attribution; these cover chat identity and tiers.
use serde_json::json;
use std::sync::Arc;

const ACTIONS: &[&str] = &[
    "clipboard.read",
    "clipboard.write",
    "network.intercept",
    "tabs.emulate",
    "files.wait_download",
];
async fn fixture(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
        config.chrome_control.user_home = Some(dir.join("chrome-home"));
    })
    .await;
    for chat in ["a", "b"] {
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":chat},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        core.shared.access.observe_request(
            "session/agent/update",
            chat,
            &json!({"agentName":"auto-approve"}),
        );
        core.shared
            .chrome_control
            .tasks
            .lock()
            .unwrap()
            .insert(format!("session-{chat}"), chat.into());
    }
    core
}
fn status() -> serde_json::Value {
    json!({"structuredContent":{"sessions":[{"sessionId":"session-a","profileId":"p"}],"profiles":[{"profileId":"p","connected":true,"extensionVersion":"0.1.6","features":ACTIONS,"extensionCapabilities":ACTIONS}]}})
}
mod r {
    use super::*;
    #[tokio::test]
    async fn clipboard_tier_step_lease() {
        let core = fixture("chrome-security-clipboard_tier_step_lease").await;
        let weak = Arc::downgrade(&core.shared);
        let _seen = core.shared.chrome_control.owner.answering(move |tool, _| {
            if tool == "chrome_status" { return status(); }
            weak.upgrade().unwrap().access.observe_request("session/agent/update", "a", &json!({"agentName":"accept-edits"}));
            json!({"structuredContent":{"status":"completed","data":{"text":"late private clipboard"}}})
        });
        for action in ["clipboard.read", "clipboard.write"] {
            for mode in ["ask", "accept-edits"] {
                core.shared.access.observe_request(
                    "session/agent/update",
                    "a",
                    &json!({"agentName":mode}),
                );
                let authorization = super::super::check::authorize(
                    &core.shared,
                    "a",
                    "chrome_execute",
                    &json!({"action":action}),
                );
                let reason = authorization.unwrap_err();
                assert!(
                    reason.contains(vw_ask::chrome::Tier::FullAccess.label()),
                    "{reason}"
                );
            }
            core.shared.access.observe_request(
                "session/agent/update",
                "a",
                &json!({"agentName":"auto-approve"}),
            );
            let result = super::super::gateway::call(
                &core.shared,
                "a",
                "chrome_execute",
                json!({"sessionId":"session-a","action":action,"args":{"tabId":7,"text":"x"}}),
            )
            .await;
            assert!(
                result.is_err(),
                "late clipboard receipt after tier removal: {result:?}"
            );
        }
        core.shutdown().await;
    }
    #[tokio::test]
    async fn cross_chat_and_person_tab_refused() {
        let core = fixture("chrome-security-cross_chat_and_person_tab_refused").await;
        let seen = core
            .shared
            .chrome_control
            .owner
            .answering(|_, _| panic!("foreign chat must never reach helper"));
        for action in ACTIONS {
            let reason = super::super::gateway::call(
                &core.shared,
                "a",
                "chrome_execute",
                json!({"sessionId":"session-b","action":action,"args":{"tabId":7,"downloadId":42}}),
            )
            .await
            .unwrap_err();
            assert!(reason.contains("another chat"), "{reason}");
        }
        assert!(seen.lock().unwrap().is_empty());
        core.shutdown().await;
    }
    #[tokio::test]
    async fn emulation_cross_chat_refused() {
        let core = fixture("chrome-security-emulation_cross_chat_refused").await;
        for (mode, allowed) in [
            ("ask", true),
            ("accept-edits", true),
            ("auto-approve", true),
        ] {
            core.shared.access.observe_request(
                "session/agent/update",
                "a",
                &json!({"agentName":mode}),
            );
            assert_eq!(
                super::super::check::authorize(
                    &core.shared,
                    "a",
                    "chrome_execute",
                    &json!({"action":"tabs.emulate"})
                )
                .is_ok(),
                allowed
            );
        }
        assert!(
            super::super::gateway::validate_session(
                &core.shared,
                "b",
                "chrome_execute",
                &json!({"sessionId":"session-a"})
            )
            .is_err()
        );
        core.shutdown().await;
    }
    #[tokio::test]
    async fn emulation_cleared_on_stop_disconnect() {
        let core = fixture("chrome-security-emulation_cleared_on_stop_disconnect").await;
        crate::sessions::observe(
            &core.shared,
            "session/agent/update",
            &json!({"sessionId":"a","agentName":"ask"}),
            &json!({}),
        )
        .await
        .unwrap();
        let tasks = core.shared.chrome_control.tasks.lock().unwrap().clone();
        assert!(
            !tasks.contains_key("session-a"),
            "mode reduction must stop persistent overrides"
        );
        assert!(
            tasks.contains_key("session-b"),
            "another chat keeps its own lease"
        );
        core.shutdown().await;
    }
    #[tokio::test]
    async fn mock_redirect_local_network_allowed() {
        let core = fixture("chrome-security-mock_redirect_local_network_allowed").await;
        let seen = core.shared.chrome_control.owner.answering(|tool, _| {
            if tool == "chrome_status" {
                status()
            } else {
                json!({"structuredContent":{"status":"completed"}})
            }
        });
        for rule in [
            json!({"pattern":"*","action":"redirect","url":"http://172.16.0.2/private"}),
            json!({"pattern":"*","action":"fulfill","status":302,"headers":[{"name":"Location","value":"http://127.0.0.1:80/private"}],"body":""}),
        ] {
            let result = super::super::gateway::call(&core.shared,"a","chrome_execute",json!({"sessionId":"session-a","action":"network.intercept","args":{"tabId":7,"rules":[rule]}})).await;
            assert!(
                result.is_ok(),
                "local mock destination must reach the helper: {result:?}"
            );
        }
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(name, _)| name == "chrome_execute")
        );
        core.shutdown().await;
    }
}
