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

/// PREDICT: the browser name "firefox" stayed as the requested profile id; the helper inventory lists only the owned
/// window `firefox:agent`, so the open was refused "Firefox is unavailable … ask the user to reconnect" although the
/// window was open. The name now opens the owned window, the only Firefox the agent can use.
#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn the_firefox_name_opens_the_owned_window() {
    for name in ["firefox", "Firefox", vw_vocabulary::texts::FIREFOX_NAME] {
        let core = chat_with_session().await;
        let shared = &core.shared;
        let seen = super::super::test_support::answering(shared, supported_helper("firefox:agent"));
        *shared.chrome_control.owner.firefox_pairing.lock().unwrap() =
            (super::super::PairingState::WindowOpen, None);
        let opened = super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":name}),
        )
        .await
        .unwrap_or_else(|refused| panic!("{name}: {refused}"));
        assert_eq!(
            opened["vwMcpResult"]["structuredContent"]["sessionId"], "held",
            "{name}: {opened}"
        );
        let sent = opens(&seen);
        assert_eq!(sent.len(), 1, "{name}");
        assert_eq!(sent[0]["profileId"], "firefox:agent", "{name}");
        core.shutdown().await;
    }
}

/// Guard: without the owned window the name keeps the existing answer (open the window in Settings > Browser > Your
/// Firefox) and nothing is opened; the name's mapping to firefox:agent does not change that.
#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn the_firefox_name_without_the_window_says_how_to_open_it() {
    let core = chat_with_session().await;
    let shared = &core.shared;
    let seen = super::super::test_support::answering(shared, |tool, args| {
        if tool == "chrome_status" {
            json!({"structuredContent":{"profiles":[{"profileId":"chrome:12345678-1234-4234-8234-123456789abc","connected":true}]}})
        } else {
            helper("held")(tool, args)
        }
    });
    let answer = super::super::gateway::call(
        shared,
        "s",
        "chrome_session",
        json!({"action":"open","profileId":"firefox"}),
    )
    .await
    .unwrap();
    let text = answer["vwMcpResult"]["structuredContent"]["error"]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("Open window"), "{answer}");
    assert_eq!(opens(&seen), Vec::<Value>::new());
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

/// PREDICT: with the person's add-on paired (a connected `firefox:<uuid>` identity in the inventory) next to the owned
/// window, the add-on identity was the only Firefox row, so it became the implicit default and `firefox:agent` / `firefox`
/// were refused "Firefox is the default for new chats". The add-on is push-only: it is never an agent-controllable
/// profile, never a default, never listed in `profileChoices`; both names open the owned window.
#[cfg(feature = "your-firefox")]
#[tokio::test]
async fn a_paired_addon_never_stands_between_the_agent_and_its_own_firefox() {
    const ADDON: &str = "firefox:12345678-1234-4234-8234-123456789abc";
    for name in ["firefox:agent", "firefox"] {
        let core = chat_with_session().await;
        let shared = &core.shared;
        let owned = supported_helper("firefox:agent");
        let seen = super::super::test_support::answering(shared, move |tool, args| {
            let mut result = owned(tool, args);
            if tool == "chrome_status" {
                result["structuredContent"]["profiles"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"profileId":ADDON,"connected":true}));
            }
            result
        });
        *shared.chrome_control.owner.firefox_pairing.lock().unwrap() =
            (super::super::PairingState::WindowOpen, None);
        let opened = super::super::gateway::call(
            shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":name}),
        )
        .await
        .unwrap_or_else(|refused| panic!("{name}: {refused}"));
        assert_eq!(
            opened["vwMcpResult"]["structuredContent"]["sessionId"], "held",
            "{name}: {opened}"
        );
        let sent = opens(&seen);
        assert_eq!(sent.len(), 1, "{name}");
        assert_eq!(sent[0]["profileId"], "firefox:agent", "{name}");
        let status = super::super::gateway::call(shared, "s", "chrome_status", json!({}))
            .await
            .unwrap();
        let choices = status["vwMcpResult"]["structuredContent"]["profileChoices"].to_string();
        assert!(!choices.contains(ADDON), "{name}: {choices}");
        assert!(choices.contains("firefox:agent"), "{name}: {choices}");
        core.shutdown().await;
    }
}
