//! Close consent uses exact tab metadata, per-call capabilities and the existing turn/session fences.
use super::read_consent_tests::{core, wait_for_card};
use super::{output_tests, test_support};
use crate::{Origin, ask::tool_calls};
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
async fn close_tabs_in_ask_wait_for_one_card_with_the_exact_tab_and_capability() {
    let mut profiles = vec!["chrome:p"];
    if cfg!(feature = "your-edge") {
        profiles.push("edge:p");
    }
    if cfg!(feature = "your-safari") {
        profiles.push("safari:p");
    }
    for profile in profiles {
        for action in ["tabs.close_owned", "tabs.close_user"] {
            for decision in ["approve", "deny"] {
                let core = connected_core(
                    profile,
                    &format!("browser-close-{profile}-{action}-{decision}"),
                )
                .await;
                let seen = test_support::answering(&core.shared, move |tool, args| {
                    if args.get("_closeTab").is_some() {
                        output_tests::mcp(
                            json!({"tab":{"tabId":7,"windowId":3,"title":"","url":"about:blank","kind":"blank tab"}}),
                        )
                    } else if tool == "chrome_status" {
                        let mut reply = output_tests::status(args);
                        reply["structuredContent"]["profiles"][0]["profileId"] = json!(profile);
                        reply["structuredContent"]["profiles"][0]["extensionVersion"] =
                            json!("0.1.8");
                        reply["structuredContent"]["sessions"][0]["profileId"] = json!(profile);
                        reply
                    } else {
                        output_tests::mcp(json!({"status":"completed","data":{"closed":true}}))
                    }
                });
                let args = json!({"sessionId":"t","action":action,"args":{"tabId":7}});
                let shared = core.shared.clone();
                let input = args.clone();
                let task = tokio::spawn(async move {
                    crate::agent_tools::prepare(
                        &shared,
                        "s",
                        "s",
                        "close-call",
                        "chrome_execute",
                        &mut input.clone(),
                    )
                    .await
                });
                let id = wait_for_card(&core.shared, &task).await;
                let ask = core.shared.asks.get(&format!("callback:{id}")).unwrap();
                assert_eq!(ask.offers, vec![vw_ask::Choice::Once, vw_ask::Choice::No]);
                let snapshot = crate::sessions::snapshot(&core.shared, "s").unwrap();
                let cards = snapshot.state["history"].as_array().unwrap();
                assert_eq!(cards[0]["detail"]["effect"]["input"], args);
                assert_eq!(
                    cards[0]["detail"]["effect"]["display"]["tab"]["kind"],
                    "blank tab"
                );
                assert!(
                    !seen
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(tool, _)| tool == "chrome_execute")
                );
                tool_calls::answer_by(
                    &core.shared,
                    Origin::Ui,
                    "s",
                    &id,
                    &json!({"decision":{"type":decision}}),
                    vw_ask::AnsweredBy::Person,
                )
                .unwrap();
                let result = task.await.unwrap();
                if decision == "deny" {
                    assert!(result.is_err());
                } else {
                    let answer = result.unwrap().expect("prepared close").await.unwrap();
                    assert!(
                        answer.to_string().contains("closed"),
                        "{profile} {action}: {answer}"
                    );
                    let calls = seen.lock().unwrap();
                    let (_, call) = calls
                        .iter()
                        .find(|(tool, _)| tool == "chrome_execute")
                        .unwrap();
                    let caps = call["_approvedCapabilities"].as_array().unwrap();
                    assert!(caps.contains(&json!(action)));
                    assert!(!caps.contains(&json!("page.edit")));
                    assert!(!caps.contains(&json!("raw.execute")));
                }
                core.shutdown().await;
            }
        }
    }
}

#[tokio::test]
async fn ask_close_reopens_its_turn_ended_session_before_the_card() {
    for batch in [false, true] {
        let core = core(&format!("close-reopens-ended-turn-{batch}")).await;
        let seen = test_support::answering(&core.shared, |tool, args| {
            if args.get("_closeTab").is_some() {
                output_tests::mcp(json!({"tab":{"tabId":7,"title":"Blank","kind":"blank tab"}}))
            } else if tool == "chrome_status" {
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
        let shared = core.shared.clone();
        let task = tokio::spawn(async move {
            let mut input = if batch {
                json!({"sessionId":"t","steps":[{"action":"tabs.close_user","args":{"tabId":7}}]})
            } else {
                json!({"sessionId":"t","action":"tabs.close_user","args":{"tabId":7}})
            };
            crate::agent_tools::prepare(
                &shared,
                "s",
                "s",
                "close-later",
                "chrome_execute",
                &mut input,
            )
            .await
        });
        let id = wait_for_card(&core.shared, &task).await;
        tool_calls::answer_by(
            &core.shared,
            Origin::Ui,
            "s",
            &id,
            &json!({"decision":{"type":"approve"}}),
            vw_ask::AnsweredBy::Person,
        )
        .unwrap();
        let answer = task.await.unwrap().unwrap().unwrap().await.unwrap();
        assert!(answer.to_string().contains("sessionReopened"), "{answer}");
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|(tool, args)| tool == "chrome_execute" && args["sessionId"] == "fresh")
        );
        core.shutdown().await;
    }
}
