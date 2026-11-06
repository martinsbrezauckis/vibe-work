//! Host chat titles remain local UI metadata across every helper-result representation; page titles remain data.
use super::{content, gateway, owner::Answer};
use serde_json::json;

#[test]
fn page_values_with_session_shaped_objects_keep_their_titles() {
    let page = json!({"sessionId":"web-session","state":"active","title":"Public document"});
    let result = content::for_model(Answer::Helper(json!({"structuredContent": {
        "sessionId":"host-session","status":"completed","data":{"value":page}
    }})));
    assert_eq!(
        result["structuredContent"]["untrustedPageContent"]["data"]["value"],
        page
    );
}

#[test]
fn session_titles_never_enter_model_content_or_structured_results() {
    let session = json!({"sessionId":"t","state":"active","title":"Private chat title","purpose":"Task","target":{"tabId":1,"title":"Public page title"}});
    for payload in [
        session.clone(),
        json!({"sessions":[session.clone()],"pending":[session.clone()]}),
        json!({"error":{"code":"SESSION_NOT_READY"},"task":session}),
    ] {
        for structured in [true, false] {
            let text = payload.to_string();
            let mut result =
                json!({"content":[{"type":"text","text":text},{"type":"text","text":text}]});
            if structured {
                result["structuredContent"] = payload.clone();
            }
            let model = content::for_model(Answer::Helper(result)).to_string();
            assert!(
                !model.contains("Private chat title"),
                "title leaked: {model}"
            );
            assert!(
                model.contains("Public page title"),
                "page title was removed: {model}"
            );
        }
    }
}

#[tokio::test]
async fn open_get_and_status_hide_titles_but_keep_the_private_title_request() {
    let core = crate::offline_core::start_with("chrome-title-output", |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s","title":"Private chat title"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"auto-approve"}),
    );
    let seen = super::test_support::answering(&core.shared, |tool, _| {
        let session =
            json!({"sessionId":"t","profileId":"p","state":"active","title":"Private chat title"});
        let payload = if tool == "chrome_status" {
            json!({"supportsSessionTitle":true,"profiles":[{"profileId":"p","connected":true,"extensionCapabilities":["session.title"]}],"sessions":[session]})
        } else {
            session
        };
        json!({"structuredContent":payload,"content":[{"type":"text","text":payload.to_string()}]})
    });
    for (tool, args) in [
        ("chrome_session", json!({"action":"open","profileId":"p"})),
        ("chrome_session", json!({"action":"get","sessionId":"t"})),
        ("chrome_status", json!({})),
    ] {
        let model = gateway::call(&core.shared, "s", tool, args).await.unwrap();
        assert!(
            !model.to_string().contains("Private chat title"),
            "{tool} leaked: {model}"
        );
    }
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(tool, args)| tool == "chrome_session"
                && args["_chatTitle"] == "Private chat title")
    );
    core.shutdown().await;
}
