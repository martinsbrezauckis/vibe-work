//! Scope, expiry and replay tests for frozen app receipts; runs with a mock helper and no browser.
use super::super::{gateway, output_tests, test_support};
use super::*;

async fn fixture(label: &str) -> (crate::core::Core, Value) {
    let core = output_tests::core(label).await;
    test_support::answering(&core.shared, |_, _| {
        output_tests::mcp(json!({
        "operationId":"input","sessionId":"t","status":"completed","action":"page.type",
        "data":{"readId":"r","target":{"ref":"input","value":"😀\"\\".repeat(6000)},
            "changes":{"complete":false,"text":"Observed validation"},"continuation":{
                "helper":{"action":"page.read","args":{"cursor":"helper-original"}}}}}))
    });
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.type","args":{"ref":"input","text":"x"}}),
    )
    .await
    .unwrap();
    let next = &answer["vwMcpResult"]["structuredContent"]["continuation"];
    let request = json!({"sessionId":"t","action":next["action"],"args":next["args"]});
    (core, request)
}

#[tokio::test]
async fn continuation_is_repeatable_but_bound_to_chat_session_and_original_permission() {
    let (core, request) = fixture("chrome-cache-scope").await;
    let first = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    let again = gateway::call(&core.shared, "s", "chrome_execute", request.clone())
        .await
        .unwrap();
    assert_eq!(first, again);
    let mut other_session = request.clone();
    other_session["sessionId"] = json!("t2");
    assert!(
        gateway::call(&core.shared, "s", "chrome_execute", other_session)
            .await
            .is_err()
    );
    assert!(
        resume(&core.shared, "another-chat", &request)
            .unwrap()
            .is_err()
    );
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    let reason = gateway::call(&core.shared, "s", "chrome_execute", request)
        .await
        .unwrap_err();
    assert!(
        reason.contains(&vw_ask::chrome::Tier::AcceptEdits.required_label()),
        "{reason}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn frozen_diagnostics_keep_the_host_reason_on_every_page() {
    let core = output_tests::core("chrome-cache-host-error").await;
    test_support::answering(&core.shared, |_, _| {
        output_tests::mcp(json!({
            "status":"failed","readId":"r","error":{"code":"INVALID_ARGUMENT","message":"A cursor already binds filters."},
            "data":{"text":"Page content ".repeat(1000)}
        }))
    });
    let mut args = json!({"sessionId":"t","action":"page.read","args":{}});
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 64, "continuation did not finish");
        let answer = gateway::call(&core.shared, "s", "chrome_execute", args.clone())
            .await
            .unwrap();
        let body = &answer["vwMcpResult"]["structuredContent"];
        assert_eq!(
            body["error"],
            json!({"code":"INVALID_ARGUMENT","message":"The browser request has invalid arguments. Check the action's arguments and try again."})
        );
        assert!(super::super::output::fits(body, 4000));
        if body["continuation"].is_null() {
            break;
        }
        args["args"] = body["continuation"]["args"].clone();
    }
    assert!(pages > 1);
    core.shutdown().await;
}

#[tokio::test]
async fn expired_or_stopped_receipts_cannot_resume_and_never_dispatch_input() {
    let (core, request) = fixture("chrome-cache-expiry").await;
    core.shared
        .chrome_control
        .output_cache
        .lock()
        .unwrap()
        .entries[0]
        .created = Instant::now().checked_sub(TTL).unwrap();
    let seen =
        test_support::answering(&core.shared, |_, _| panic!("expired buffer reached helper"));
    let reason = gateway::call(&core.shared, "s", "chrome_execute", request)
        .await
        .unwrap_err();
    assert!(reason.contains("expired") && reason.contains("fresh read"));
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;

    let (core, request) = fixture("chrome-cache-stop").await;
    core.shared.chrome_control.end_chat("s", false);
    assert!(
        core.shared
            .chrome_control
            .output_cache
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(resume(&core.shared, "s", &request).unwrap().is_err());
    core.shutdown().await;
}

#[tokio::test]
async fn clamped_unicode_receipts_reconstruct_without_shortening_fragment_text() {
    let core = output_tests::core("chrome-cache-clamp").await;
    let raw = json!({"status":"completed","action":"page.read","data":{"readId":"r",
        "refs":[{"ref":"e","options":[{"value":"https://a.test/opaque?q=😀\"\\".repeat(2000),"text":"Choice"}]}]}});
    let copy = raw.clone();
    test_support::answering(&core.shared, move |_, _| output_tests::mcp(copy.clone()));
    let reply = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{"maxChars":60000,"maxBytes":64000}}),
    )
    .await
    .unwrap();
    let restored = super::super::observation_output_tests::reconstruct(&core.shared, reply).await;
    assert_eq!(restored["untrustedPageContent"], raw);
    assert!(restored["requestLimit"].as_str().unwrap().contains("4000"));
    core.shutdown().await;
}

#[tokio::test]
async fn batch_buffer_remains_bound_to_the_same_session() {
    let core = output_tests::core("chrome-cache-retained").await;
    let mut result = json!({"structuredContent":{"status":"completed",
        "steps":[{"step":1,"result":{"status":"completed","operationId":"op",
            "untrustedPageContent":{"data":{"readId":"r","text":"x".repeat(15000)}}}}]}});
    bound(
        &core.shared,
        "s",
        &json!({"sessionId":"t","steps":[{"action":"page.read","args":{}}]}),
        &mut result,
    );
    let body = &result["structuredContent"];
    let next = &body["continuation"];
    let reply = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":next["action"],"args":next["args"]}),
    )
    .await;
    assert!(reply.is_ok(), "{reply:?}");
    core.shutdown().await;
}

#[tokio::test]
async fn an_ended_buffer_does_not_reopen_browser_work() {
    let (core, request) = fixture("chrome-cache-ended").await;
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "chrome:p", None);
    core.shared.chrome_control.end_turn("s");
    let seen = test_support::answering(&core.shared, |_, _| {
        output_tests::mcp(json!({"sessionId":"new", "state":"active", "profileId":"chrome:p"}))
    });
    let reply = gateway::call(&core.shared, "s", "chrome_execute", request).await;
    assert!(reply.is_err());
    assert!(
        seen.lock().unwrap().is_empty(),
        "a frozen receipt must not reopen a browser session"
    );
    core.shutdown().await;
}
