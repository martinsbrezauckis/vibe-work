//! Mixed-browser gateway regressions using trusted helper replies; no desktop or real browser.
use super::{AddonState, commands, edge_agent_tests::fixture, gateway, owner, test_support};
use serde_json::{Value, json};

const CHROME: &str = "chrome:profile";
const EDGE: &str = "edge:profile";

#[allow(
    clippy::unwrap_used,
    reason = "test fixture setup must fail the test on an error"
)]
async fn both(name: &str) -> crate::Core {
    let core = fixture(
        name,
        json!([
            {"profileId":CHROME,"connected":true,"extensionVersion":"0.1.3"},
            {"profileId":EDGE,"connected":true,"extensionVersion":"0.1.3"}
        ]),
    )
    .await;
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_session" && args["action"] == "open" {
            let id = if args["profileId"] == EDGE {
                "edge-task"
            } else {
                "chrome-task"
            };
            json!({"structuredContent":{"sessionId":id,"profileId":args["profileId"],"state":"active"}})
        } else {
            json!({"structuredContent":{"sessionId":args["sessionId"],"status":"completed","data":{}}})
        }
    });
    core.shared
        .chrome_control
        .owner
        .tools
        .lock()
        .unwrap()
        .push(json!({"name":"chrome_status"}));
    std::fs::write(
        core.shared.chrome_control.dir().unwrap().join("setup.json"),
        json!({"files":[],"browsers":["Google Chrome","Microsoft Edge"],"userData":[]}).to_string(),
    )
    .unwrap();
    for profile in [CHROME, EDGE] {
        gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":profile}),
        )
        .await
        .unwrap();
    }
    core
}

fn body(answer: &Value) -> &Value {
    &answer["vwMcpResult"]["structuredContent"]
}

#[tokio::test]
async fn calls_by_id_keep_both_browsers_and_their_own_refusals() {
    let core = both("session-id-browser-routing").await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"other"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "other",
        &json!({"agentName":"auto-approve"}),
    );
    for (id, profile) in [("chrome-task", CHROME), ("edge-task", EDGE)] {
        assert_eq!(
            super::agent_browser::selected(
                &core.shared,
                "s",
                "chrome_execute",
                &json!({"sessionId":id})
            ),
            super::agent_browser::Browser::from_profile(profile)
        );
        for (tool, args) in [
            (
                "chrome_execute",
                json!({"sessionId":id,"action":"page.read","args":{}}),
            ),
            (
                "chrome_execute",
                json!({"sessionId":id,"steps":[{"action":"page.read","args":{}}]}),
            ),
            ("chrome_session", json!({"sessionId":id,"action":"get"})),
            ("chrome_session", json!({"sessionId":id,"action":"cancel"})),
            ("chrome_session", json!({"sessionId":id,"action":"stop"})),
        ] {
            let result = gateway::call(&core.shared, "s", tool, args).await.unwrap();
            assert!(!result.to_string().contains("addon"), "{id}: {result}");
        }
        let refused = gateway::call(
            &core.shared,
            "other",
            "chrome_execute",
            json!({"sessionId":id,"action":"page.read","args":{}}),
        )
        .await
        .unwrap_err();
        let browser = if profile == EDGE { "Edge" } else { "Chrome" };
        assert_eq!(
            refused,
            format!("Not run: this {browser} session belongs to another chat.")
        );
    }
    // Availability and ownership refusals name the id's browser even after selecting the other one.
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[
            {"profileId":CHROME,"connected":false}, {"profileId":EDGE,"connected":true}
        ]}),
    );
    let answer = gateway::call(
        &core.shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"chrome-task","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    assert!(
        body(&answer)["addon"]["instruction"]
            .as_str()
            .unwrap()
            .contains("Chrome"),
        "{answer}"
    );
    assert!(!answer.to_string().contains("Edge"), "{answer}");
    core.shutdown().await;
}

#[tokio::test]
async fn an_idle_chat_keeps_the_stopped_browsers_state_separate() {
    for (stopped, other, stopped_profile, name) in [
        ("chrome-task", "edge-task", CHROME, "Chrome"),
        ("edge-task", "chrome-task", EDGE, "Edge"),
    ] {
        let core = both(&format!("session-browser-stop-{name}")).await;
        owner::update_work_for_test(
            &core.shared,
            &json!({"structuredContent":{"sessions":[
                {"sessionId":stopped,"state":"stopped"}, {"sessionId":other,"state":"active"}
            ]}}),
        );
        let refused = gateway::call(
            &core.shared,
            "s",
            "chrome_session",
            json!({"action":"open","profileId":stopped_profile}),
        )
        .await
        .unwrap_err();
        assert!(
            refused.contains(&format!("you pressed Stop in Your {name}")),
            "{refused}"
        );
        let result = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":other,"action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert_eq!(body(&result)["status"], "completed", "{result}");
        core.shutdown().await;
    }
}

#[tokio::test]
async fn turn_ended_ids_reopen_their_own_browser_after_switching() {
    let core = both("session-browser-reopen").await;
    super::end_turn(&core.shared, "s");
    for (id, profile) in [("chrome-task", CHROME), ("edge-task", EDGE)] {
        let result = gateway::call(
            &core.shared,
            "s",
            "chrome_execute",
            json!({"sessionId":id,"action":"page.read","args":{}}),
        )
        .await
        .unwrap();
        assert_eq!(
            body(&result)["sessionReopened"]["sessionId"],
            id,
            "{profile}: {result}"
        );
    }
    core.shutdown().await;
}

#[tokio::test]
async fn connected_edge_never_inherits_chromes_external_install_state() {
    let core = both("connected-edge-install-state").await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::write(
        dir.join("extension-install.json"),
        json!({"file":null,"key":null,"chrome_pids":["1234"]}).to_string(),
    )
    .unwrap();
    assert!(commands::view(&core.shared).setup.install_state.is_some());
    let edge = commands::view(&core.shared).edge.unwrap();
    assert_eq!(edge.addon.as_ref().unwrap().state, AddonState::Connected);
    assert_eq!(
        edge.install_state, None,
        "Edge does not use Chrome's external installer"
    );
    assert_eq!(
        edge.browser_issue, None,
        "Chrome's process probe cannot describe Edge"
    );
    let text = serde_json::to_string(&edge).unwrap();
    assert!(!text.contains("Chrome"), "{text}");
    core.shutdown().await;
}

#[tokio::test]
async fn identity_mismatch_survives_empty_status_polls_for_both_browsers() {
    let core = both("browser-identity-mismatch-card").await;
    let dir = core.shared.chrome_control.dir().unwrap();
    std::fs::write(
        dir.join("extension-install.json"),
        json!({"file":null,"key":null,"chrome_pids":[]}).to_string(),
    )
    .unwrap();
    for profile in [CHROME, EDGE] {
        owner::observe(&std::sync::Arc::downgrade(&core.shared), &json!({
            "method":"notifications/owner/pairing", "params":{"profileId":profile,"state":"mismatch"}
        })).unwrap();
    }
    for profiles in [
        json!([]),
        json!([
            {"profileId":CHROME,"connected":false}, {"profileId":EDGE,"connected":false}
        ]),
        json!([]),
    ] {
        test_support::observe_status(&core.shared, &json!({"profiles":profiles}));
        let view = commands::view(&core.shared);
        for (setup, browser) in [(view.setup, "Chrome"), (*view.edge.unwrap(), "Edge")] {
            let addon = setup.addon.unwrap();
            assert_eq!(addon.state, AddonState::OtherOwner, "{browser}: {addon:?}");
            assert!(addon.instruction.contains(&format!("another Vibe Work. Ask the person to click Open pairing page and choose Replace paired Vibe Work in {browser}")), "{addon:?}");
            assert_eq!(addon.step, None);
        }
    }
    test_support::observe_status(
        &core.shared,
        &json!({"profiles":[
            {"profileId":CHROME,"connected":true}, {"profileId":EDGE,"connected":true}
        ]}),
    );
    let view = commands::view(&core.shared);
    assert_eq!(view.setup.addon.unwrap().state, AddonState::Connected);
    assert_eq!(
        view.edge.unwrap().addon.unwrap().state,
        AddonState::Connected
    );
    core.shutdown().await;
}
