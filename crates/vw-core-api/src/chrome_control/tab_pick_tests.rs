//! On the app side: a tab the person picks in a chat's composer is only read for it
//! (no grant: the agent reaches the person's tabs); the local-network switch reaches Chrome's live tabs; the agent
//! cannot send the app-only helper arguments; the agent's tab metadata never shows a local-network page's title.
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

/// A pick in a chat's composer gives the chat the tab through the helper's grant
/// (`_grantTab`). After the owner's rule (no tab restriction: the agent reaches the person's tabs anyway) the picker
/// only reads the picked tab's context for the composer: nothing is granted, whatever chat the window names.
#[tokio::test]
async fn a_picked_tab_is_only_read_for_the_composer() {
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
    // The window as it sent these in earlier 0.1.4 builds (naming the chat; now not a valid request), and as it sends
    // them now.
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
    core.shutdown().await;
}

/// PREDICT (before the change): the switch is saved for the next call only; nothing reaches Chrome's live tabs.
#[tokio::test]
async fn turning_local_network_pages_off_reaches_chrome() {
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
                json!({"_netPolicyChange":{"localNetwork":false,"generation":2}})
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

/// PREDICT (before the change): the agent's tabs.list shows a local-network page's title.
#[tokio::test]
async fn the_agents_tab_list_shows_no_local_network_title() {
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
    assert!(!text.contains("Router admin secrets"), "{text}");
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

/// The same projection for any tab metadata a Chrome answer carries (`tabs.list`, `tabs.inspect_other`, a created or
/// opened tab, a session's target), with a name that resolves to the local network.
#[tokio::test]
async fn every_tab_record_in_an_answer_drops_a_local_title() {
    let names = |name: String| async move {
        if name == "nas.example" {
            vw_preview::NameVerdict::LocalNetwork("private".into())
        } else {
            vw_preview::NameVerdict::Public
        }
    };
    let mut answer = json!({"structuredContent":{"data":{
        "tabs":[{"tabId":1,"url":"https://nas.example/files","title":"NAS files"},{"tabId":2,"url":"http://[fd00::1]/","title":"ULA"}],
        "tab":{"tabId":3,"url":"http://10.0.0.2/","title":"Ten"},
        "target":{"tabId":4,"url":"https://example.com/","title":"Public"}
    }}});
    super::tabs::hide_local_titles(&mut answer, names).await;
    let data = &answer["structuredContent"]["data"];
    assert_eq!(data["tabs"][0]["title"], "");
    assert_eq!(data["tabs"][1]["title"], "");
    assert_eq!(data["tab"]["title"], "");
    assert_eq!(data["target"]["title"], "Public");
    assert_eq!(data["tabs"][0]["url"], "https://nas.example/files");
}

/// PREDICT (before the change): a helper answer that carries the tab list only as text (no structuredContent), or a
/// record without a tabId or with a pendingUrl, keeps the local title.
#[tokio::test]
async fn a_local_title_in_the_text_part_or_a_loose_record_is_hidden_too() {
    let names = |_: String| async { vw_preview::NameVerdict::Public };
    let mut text_only = json!({"content":[{"type":"text","text":json!({"tabs":[{"tabId":1,"url":"http://172.20.3.4/","title":"Router admin secrets"}]}).to_string()}]});
    super::tabs::hide_local_titles(&mut text_only, names).await;
    assert!(
        !text_only.to_string().contains("Router admin secrets"),
        "{text_only}"
    );
    let mut loose = json!({"structuredContent":{"data":{"opened":{"url":"http://10.0.0.2/","title":"NAS"},"pending":{"pendingUrl":"http://[fd00::2]/","url":"","title":"ULA"}}}});
    super::tabs::hide_local_titles(&mut loose, names).await;
    assert!(!loose.to_string().contains("\"NAS\""), "{loose}");
    assert!(!loose.to_string().contains("\"ULA\""), "{loose}");
}

/// PREDICT (before the change): a matched record is not searched further, so a tab record nested in it (an opener
/// tab, a window's or group's tabs) keeps its local title.
#[tokio::test]
async fn a_tab_record_nested_in_another_loses_its_local_title() {
    let names = |_: String| async { vw_preview::NameVerdict::Public };
    let mut answer = json!({"structuredContent":{"data":{"tab":{"tabId":1,"url":"https://example.com/","title":"Public",
        "openerTab":{"tabId":2,"url":"http://10.0.0.9/","title":"Opener on the LAN"},
        "group":{"tabs":[{"tabId":3,"url":"http://172.20.3.9/","title":"Grouped LAN"}]}}}}});
    super::tabs::hide_local_titles(&mut answer, names).await;
    let text = answer.to_string();
    assert!(text.contains("\"Public\""), "{text}");
    assert!(
        !text.contains("Opener on the LAN") && !text.contains("Grouped LAN"),
        "{text}"
    );
}
