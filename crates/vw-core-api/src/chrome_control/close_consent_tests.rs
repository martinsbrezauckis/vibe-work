//! Close consent uses exact tab metadata, per-call capabilities and the existing turn/session fences.
use super::read_consent_tests::core;
use super::{output_tests, test_support};
use serde_json::json;

async fn connected_core(profile: &str, label: &str) -> crate::Core {
    let core = core(label).await;
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", profile, None);
    if profile.starts_with("safari:") {
        std::fs::create_dir_all(core.shared.chrome_control.dir().unwrap()).unwrap();
        std::fs::write(
            core.shared
                .chrome_control
                .dir()
                .unwrap()
                .join("safari-enabled.json"),
            "true",
        )
        .unwrap();
    }
    core.shared
        .chrome_control
        .owner
        .edge_pairing
        .lock()
        .unwrap()
        .0 = super::PairingState::Paired;
    core.shared.chrome_control.owner.connected_profiles.observe(
        &json!({"profiles":[{"profileId":profile,"connected":true,"extensionVersion":"0.1.8"}]}),
    );
    core
}

#[tokio::test]
async fn closing_owned_and_handed_tabs_runs_without_cards_in_ask() {
    let mut profiles = vec!["chrome:p"];
    if cfg!(feature = "your-edge") {
        profiles.push("edge:p");
    }
    if cfg!(feature = "your-safari") {
        profiles.push("safari:p");
    }
    for profile in profiles {
        for action in ["tabs.close_owned", "tabs.close_user"] {
            let core = connected_core(profile, &format!("browser-close-{profile}-{action}")).await;
            let seen = test_support::answering(&core.shared, |tool, args| {
                if tool == "chrome_status" {
                    output_tests::status(args)
                } else {
                    output_tests::mcp(json!({"status":"completed","data":{"closed":true}}))
                }
            });
            let mut args = json!({"sessionId":"t","action":action,"args":{"tabId":7}});
            let prepared = crate::agent_tools::prepare(
                &core.shared,
                "s",
                "s",
                "close-call",
                "chrome_execute",
                &mut args,
            )
            .await
            .unwrap();
            assert!(prepared.is_none());
            assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
            let answer = super::gateway::call(&core.shared, "s", "chrome_execute", args)
                .await
                .unwrap();
            assert!(
                answer.to_string().contains("closed"),
                "{profile} {action}: {answer}"
            );
            let calls = seen.lock().unwrap().clone();
            let call = &calls
                .iter()
                .find(|(tool, _)| tool == "chrome_execute")
                .unwrap()
                .1;
            let caps = call["_approvedCapabilities"].as_array().unwrap();
            assert!(caps.contains(&json!(action)));
            assert!(!caps.contains(&json!("page.commit")));
            assert!(!caps.contains(&json!("raw.execute")));
            drop(calls);
            core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn ask_close_reopens_its_turn_ended_session_without_a_card() {
    for batch in [false, true] {
        let core = core(&format!("close-reopens-ended-turn-{batch}")).await;
        let seen = test_support::answering(&core.shared, |tool, args| {
            if tool == "chrome_status" {
                output_tests::status(args)
            } else if tool == "chrome_session" && args["action"] == "open" {
                output_tests::mcp(
                    json!({"sessionId":"fresh","profileId":"chrome:p","state":"active"}),
                )
            } else {
                output_tests::mcp(json!({"status":"completed","data":{"closed":true}}))
            }
        });
        super::end_turn(&core.shared, "s");
        let args = if batch {
            json!({"sessionId":"t","steps":[{"action":"tabs.close_user","args":{"tabId":7}}]})
        } else {
            json!({"sessionId":"t","action":"tabs.close_user","args":{"tabId":7}})
        };
        let answer = super::gateway::call(&core.shared, "s", "chrome_execute", args)
            .await
            .unwrap();
        assert!(answer.to_string().contains("sessionReopened"), "{answer}");
        assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(tool, args)| tool == "chrome_execute" && args["sessionId"] == "fresh")
        );
        core.shutdown().await;
    }
}
