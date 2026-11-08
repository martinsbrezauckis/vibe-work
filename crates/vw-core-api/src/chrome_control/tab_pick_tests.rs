//! Person-picked tab context carries its receiving chat so a previously chat-owned window can transfer.
//! Legacy callers may read context without a handoff; agent tools cannot send these private helper arguments.
use serde_json::{Value, json};

use crate::{ChromeControlRequest, Origin};

async fn core_with_chrome(name: &str) -> crate::Core {
    crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await
}

fn open_chat(shared: &crate::core::Shared, session: &str, mode: &str) {
    let _ = shared
        .store
        .load(
            json!({"session":{"id":session},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    shared
        .access
        .observe_request("session/agent/update", session, &json!({"agentName":mode}));
}

fn calls(seen: &std::sync::Mutex<Vec<(String, Value)>>) -> Vec<(String, Value)> {
    seen.lock().unwrap().clone()
}

/// Picking includes the receiving chat; it does not open a browser session or execute agent work.
#[tokio::test]
async fn a_picked_tab_carries_the_receiving_chat_with_its_context_read() {
    let core = core_with_chrome("tab-pick").await;
    let shared = &core.shared;
    open_chat(shared, "s", "auto-approve");
    let seen = shared.chrome_control.owner.answering(|_, args| {
        if args.get("_tabContext").is_some() {
            json!({"structuredContent":{"requestId":"r","status":"completed","data":{"url":"https://example.com/","title":"Example","text":"Hello"}}})
        } else {
            json!({"structuredContent":{"tabs":[{"profileId":"p","profileName":"Chrome","tab":{"tabId":7,"title":"Example","url":"https://example.com/"}}]}})
        }
    });
    // A legacy read-only caller may omit the chat; the current picker names its receiving chat.
    for (tabs, pick) in [
        (
            json!({"op":"tabs","sessionId":"s"}),
            json!({"op":"tabContext","profileId":"p","tabId":7,"sessionId":"s"}),
        ),
        (
            json!({"op":"tabs"}),
            json!({"op":"tabContext","profileId":"p","tabId":7}),
        ),
    ] {
        for request in [tabs, pick] {
            if let Ok(request) = serde_json::from_value::<ChromeControlRequest>(request) {
                core.chrome_control(Origin::Ui, request).await.unwrap();
            }
        }
    }
    let sent: Vec<_> = calls(&seen).into_iter().map(|(_, args)| args).collect();
    assert!(
        sent.iter()
            .all(|args| args.get("_tabs").is_some() || args.get("_tabContext").is_some()),
        "only the list and the context read reach the helper: {sent:?}"
    );
    assert!(
        sent.iter().any(|args| args.get("_tabContext").is_some()),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|args| args["_tabContext"]["clientSessionId"] == "s")
    );
    core.shutdown().await;
}

/// PREDICT (before the change): the switch is saved for the next call only; nothing reaches Chrome's live tabs.
#[tokio::test]
async fn legacy_network_requests_keep_browser_addresses_enabled() {
    let core = core_with_chrome("tab-switch").await;
    let shared = &core.shared;
    let seen = shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"structuredContent":{"revision":1,"pendingProfiles":["p"]}}));
    for enabled in [true, false] {
        core.chrome_control(
            Origin::Ui,
            ChromeControlRequest::SetLocalNetwork { enabled },
        )
        .await
        .unwrap();
    }
    assert_eq!(
        calls(&seen),
        vec![
            (
                "chrome_status".to_owned(),
                json!({"_netPolicyChange":{"localNetwork":true,"generation":1}})
            ),
            (
                "chrome_status".to_owned(),
                json!({"_netPolicyChange":{"localNetwork":true,"generation":2}})
            ),
        ]
    );
    core.shutdown().await;
}

/// The app-only helper arguments never come from the agent: the gateway refuses them before anything reaches the
/// helper (regression; `validate_public_args` already refuses every `_` argument).
#[tokio::test]
async fn the_agent_cannot_send_the_app_only_helper_arguments() {
    let core = core_with_chrome("tab-private-args").await;
    let shared = &core.shared;
    open_chat(shared, "s", "auto-approve");
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    let seen = shared
        .chrome_control
        .owner
        .answering(|_, _| json!({"structuredContent":{"granted":true}}));
    for (tool, args) in [
        (
            "chrome_status",
            json!({"_tabContext":{"profileId":"p","tabId":7}}),
        ),
        (
            "chrome_status",
            json!({"_netPolicyChange":{"localNetwork":true}}),
        ),
        (
            "chrome_execute",
            json!({"sessionId":"t","steps":[{"_netPolicyChange":{"localNetwork":true}}]}),
        ),
    ] {
        let refused = super::gateway::call(shared, "s", tool, args.clone()).await;
        assert!(
            refused.is_err()
                || refused
                    .as_ref()
                    .is_ok_and(|v| v.to_string().contains("invalid Chrome tool arguments")),
            "{tool} {args}: {refused:?}"
        );
    }
    assert!(
        calls(&seen)
            .iter()
            .all(|(_, args)| args.get("_tabContext").is_none()
                && args.get("_netPolicyChange").is_none()),
        "{:?}",
        calls(&seen)
    );
    core.shutdown().await;
}

/// Local addresses have the same title visibility as other permitted sites.
#[tokio::test]
async fn the_agents_tab_list_keeps_local_network_titles() {
    let core = core_with_chrome("tab-titles").await;
    let shared = &core.shared;
    open_chat(shared, "s", "ask");
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    let _seen = super::test_support::answering(shared, |_, _| {
        json!({"structuredContent":{"status":"completed","action":"tabs.list","data":{"tabs":[
            {"tabId":1,"url":"http://172.20.3.4/admin","title":"Router admin secrets","active":false},
            {"tabId":2,"url":"https://example.com/","title":"Example Domain","active":true}
        ]}}})
    });
    let value = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"tabs.list","args":{}}),
    )
    .await
    .unwrap();
    let text = value.to_string();
    assert!(text.contains("Router admin secrets"), "{text}");
    assert!(text.contains("172.20.3.4"), "the address stays: {text}");
    assert!(text.contains("Example Domain"), "{text}");
    core.shutdown().await;
}

/// The helper lists non-web tabs too. Their controllability and refusal text survive the app's model envelope.
#[tokio::test]
async fn non_web_tabs_and_their_refusal_reach_the_agent() {
    const REFUSAL: &str = "Not run: no tab with this ID is available. Use a tabId from tabs.list";
    let core = core_with_chrome("tab-non-web").await;
    let shared = &core.shared;
    open_chat(shared, "s", "ask");
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    let seen = super::test_support::answering(shared, |_, args| {
        if args["action"] == "tabs.list" {
            json!({"structuredContent":{"status":"completed","action":"tabs.list","data":{"tabs":[
                {"tabId":1,"url":"about:blank","title":"","controllable":false},
                {"tabId":2,"url":"chrome://newtab/","title":"New Tab","controllable":false},
                {"tabId":3,"url":"https://example.com/","title":"Example","controllable":true}
            ]}}})
        } else {
            json!({"structuredContent":{"status":"failed","action":"page.read","error":{"message":REFUSAL}}})
        }
    });
    let listed = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"tabs.list","args":{}}),
    )
    .await
    .unwrap();
    let tabs = &listed["vwMcpResult"]["structuredContent"]["untrustedPageContent"]["data"]["tabs"];
    assert_eq!(tabs.as_array().unwrap().len(), 3, "{listed}");
    for (index, url, controllable) in [
        (0, "about:blank", false),
        (1, "chrome://newtab/", false),
        (2, "https://example.com/", true),
    ] {
        assert_eq!(tabs[index]["url"], url);
        assert_eq!(tabs[index]["controllable"], controllable);
    }
    let refused = super::gateway::call(
        shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{"tabId":1}}),
    )
    .await
    .unwrap();
    let result = &refused["vwMcpResult"];
    assert_eq!(
        result["structuredContent"]["untrustedPageContent"]["error"]["message"], REFUSAL,
        "{refused}"
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(REFUSAL)
    );
    assert!(
        calls(&seen)
            .iter()
            .any(|(_, args)| { args["action"] == "page.read" && args["args"]["tabId"] == 1 })
    );
    core.shutdown().await;
}

/// Exercise every helper answer shape through the real gateway projection.
#[tokio::test]
async fn local_titles_survive_nested_text_and_pending_tab_records() {
    let core = core_with_chrome("tab-title-shapes").await;
    let shared = &core.shared;
    open_chat(shared, "s", "ask");
    shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    let payloads = [
        json!({"structuredContent":{"data":{"tabs":[{"tabId":1,"url":"https://nas.local/files","title":"NAS files"},{"tabId":2,"url":"http://[fd00::1]/","title":"ULA"}],"tab":{"tabId":3,"url":"http://10.0.0.2/","title":"Ten"},"target":{"tabId":4,"url":"https://example.com/","title":"Public"}}}}),
        json!({"content":[{"type":"text","text":json!({"tabs":[{"tabId":1,"url":"http://172.20.3.4/","title":"Router admin"}]}).to_string()}]}),
        json!({"structuredContent":{"data":{"opened":{"url":"http://10.0.0.2/","title":"NAS"},"pending":{"pendingUrl":"http://[fd00::2]/","url":"","title":"ULA"}}}}),
        json!({"structuredContent":{"data":{"tab":{"tabId":1,"url":"https://example.com/","title":"Public","openerTab":{"tabId":2,"url":"http://10.0.0.9/","title":"Opener on the LAN"},"group":{"tabs":[{"tabId":3,"url":"http://172.20.3.9/","title":"Grouped LAN"}]}}}}}),
    ];
    for (payload, titles) in payloads.into_iter().zip([
        vec!["NAS files", "ULA", "Ten", "Public"],
        vec!["Router admin"],
        vec!["NAS", "ULA"],
        vec!["Opener on the LAN", "Grouped LAN"],
    ]) {
        let _seen = super::test_support::answering(shared, move |_, _| payload.clone());
        let answer = super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"tabs.list","args":{}}),
        )
        .await
        .unwrap();
        for title in titles {
            assert!(answer.to_string().contains(title), "{answer}");
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn picked_context_follows_the_draft_reset_and_first_message() {
    let core = core_with_chrome("picked-draft-window").await;
    let shared = &core.shared;
    open_chat(shared, "s", "auto-approve");
    shared.chrome_control.owner.connected_profiles.observe(
        &json!({"profiles":[{"profileId":"chrome:p","connected":true,"extensionVersion":"0.1.6"}]}),
    );
    let seen = shared.chrome_control.owner.answering(|_, _| json!({"structuredContent":{"requestId":"r","status":"completed","data":{"url":"https://example.com/","title":"Example","text":"Hello"}}}));
    let crate::CommandResult::NewChat { view } = core
        .new_chat(Origin::Ui, crate::NewChatRequest::State {})
        .await
        .unwrap()
    else {
        panic!("expected draft state")
    };
    let draft = view.owner_id;
    core.chrome_control(
        Origin::Ui,
        ChromeControlRequest::TabContext {
            profile_id: "chrome:p".into(),
            tab_id: 7,
            session_id: Some(draft.clone()),
        },
    )
    .await
    .unwrap();
    let crate::CommandResult::NewChat { view } = core
        .new_chat(Origin::Ui, crate::NewChatRequest::CloseAll {})
        .await
        .unwrap()
    else {
        panic!("expected renewed draft")
    };
    let renewed = view.owner_id;
    core.new_chat(
        Origin::Ui,
        crate::NewChatRequest::Adopt {
            session_id: "s".into(),
        },
    )
    .await
    .unwrap();
    let sent = calls(&seen);
    assert!(
        sent.iter()
            .any(|(_, args)| args["_tabContext"]["clientSessionId"] == draft)
    );
    assert!(
        sent.iter()
            .any(|(_, args)| args["_adoptChat"] == json!({"old":draft,"new":renewed}))
    );
    assert!(
        sent.iter()
            .any(|(_, args)| args["_adoptChat"] == json!({"old":renewed,"new":"s"}))
    );
    core.shutdown().await;
}
