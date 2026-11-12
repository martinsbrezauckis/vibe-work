//! New checked actions share the retained-session and person-question boundaries.
//! Uses the parent recording helper; no browser effect runs outside the checked gateway.
use super::*;

#[tokio::test]
async fn retained_checked_actions_wait_for_the_person_and_resume_the_same_session() {
    check_handover("p").await;
}

#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn firefox_question_handover_resumes_the_retained_profile() {
    check_handover("firefox:agent").await;
}

async fn check_handover(profile: &'static str) {
    let core = chat_with_session().await;
    let shared = &core.shared;
    let seen = super::super::test_support::answering(shared, supported_helper(profile));
    if profile == "firefox:agent" {
        *shared.chrome_control.owner.firefox_pairing.lock().unwrap() =
            (super::super::PairingState::WindowOpen, None);
    }
    let opened = super::super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":profile}),
    )
    .await
    .unwrap();
    assert_eq!(
        opened["vwMcpResult"]["structuredContent"]["sessionId"], "held",
        "{opened}"
    );
    let question = shared.asks.open(
        "user_input",
        Some("s".into()),
        vw_ask::Requester::Agent,
        vec![vw_ask::Choice::No],
        None,
    );
    super::super::end_turn(shared, "s");
    for (action, args) in [
        ("page.focus", json!({"ref":"field"})),
        (
            "page.wait_for",
            json!({"selector":"#ready","state":"visible"}),
        ),
        (
            "storage.change",
            json!({"operation":"clear","area":"local","tabId":7,"origin":"https://fixture.test"}),
        ),
    ] {
        let call = json!({"sessionId":"held","action":action,"args":args});
        let refused = super::super::gateway::call(shared, "s", "chrome_execute", call)
            .await
            .unwrap_err();
        assert_eq!(
            Some(refused.as_str()),
            shared.asks.question_hold("s"),
            "{action}"
        );
    }
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(tool, _)| tool == "chrome_execute")
    );
    shared.asks.clear_question_holds(Some("s"));
    for (action, args) in [
        ("page.focus", json!({"ref":"field"})),
        (
            "storage.change",
            json!({"operation":"clear","area":"local","tabId":7,"origin":"https://fixture.test"}),
        ),
    ] {
        let answer = super::super::gateway::call(
            shared,
            "s",
            "chrome_execute",
            json!({"sessionId":"held","action":action,"args":args}),
        )
        .await
        .unwrap();
        assert_eq!(
            answer["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{answer}"
        );
    }
    assert_eq!(opens(&seen).len(), 1);
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(tool, args)| tool == "chrome_session"
                && args["action"] == "resume"
                && args["sessionId"] == "held")
            .count(),
        1
    );
    drop(question);
    core.shutdown().await;
}

fn supported_helper(profile: &'static str) -> impl Fn(&str, &Value) -> Value + Send + 'static {
    let fallback = helper("held");
    move |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"firefoxWindow":"Window open","sessions":[{"sessionId":"held","profileId":profile,"state":"active"}],
                "profiles":[{"profileId":profile,"connected":true,"extensionVersion":"0.1.7",
                    "features":["page.focus","page.wait_for","storage.change"],"extensionCapabilities":["transport.contract.v1","firefox.page.v1","storage.change.clear"]}]}})
        } else {
            let mut result = fallback(tool, args);
            if result["structuredContent"].get("profileId").is_some() {
                result["structuredContent"]["profileId"] = json!(profile);
            }
            result
        }
    }
}
