//! Ask-mode personal browser reads wait for a scoped card before helper execution; other modes retain their policy.
use super::{output_tests, test_support};
use crate::{Origin, ask::tool_calls};
use serde_json::{Value, json};

async fn core(name: &str) -> crate::Core {
    let core = crate::offline_core::start_with(name, |config, dir| {
        config.chrome_control.package = Some(dir.join("chrome-package"));
        config.agent_tools = Some(crate::AgentToolsConfig::new(
            dir.join("tools"),
            dir.join("data"),
            None,
        ));
    })
    .await;
    let _ = core
        .shared
        .store
        .load(
            json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    core.shared
        .access
        .observe_request("session/agent/update", "s", &json!({"agentName":"ask"}));
    core.shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("t".into(), "s".into());
    core.shared
        .chrome_control
        .work
        .lock()
        .unwrap()
        .open("t", "s", "active", "chrome:p", None);
    core
}

async fn prepare(
    shared: std::sync::Arc<crate::core::Shared>,
    args: Value,
) -> Option<tool_calls::receipts::Execution> {
    crate::agent_tools::prepare(
        &shared,
        "s",
        "s",
        "call",
        "chrome_execute",
        &mut args.clone(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_personal_read_in_a_batch_waits_before_any_step_runs() {
    let core = core("browser-batch-read-consent").await;
    let seen = test_support::answering(&core.shared, |_, _| panic!("no helper before consent"));
    let args = json!({"sessionId":"t","steps":[{"action":"page.read","args":{}},{"action":"history.read","args":{}}]});
    let task = tokio::spawn(prepare(core.shared.clone(), args));
    let _ = wait_for_card(&core.shared, &task).await;
    assert!(!task.is_finished(), "batch must wait for its read card");
    assert!(seen.lock().unwrap().is_empty());
    task.abort();
    let _ = task.await;
    core.shutdown().await;
}

#[tokio::test]
async fn discovery_names_consent_for_personal_reads_in_ask() {
    let core = core("browser-read-discovery").await;
    let mut status = json!({"structuredContent":{"capabilities":{"actions":[{"action":"history.read"},{"action":"bookmarks.read"},{"action":"page.type"}]}}});
    super::check::annotate_modes(&core.shared, "s", "chrome_status", &mut status);
    assert_eq!(
        status["structuredContent"]["capabilities"]["actions"][0]["approvalRequired"],
        true
    );
    assert_eq!(
        status["structuredContent"]["capabilities"]["actions"][1]["approvalRequired"],
        true
    );
    assert!(
        status["structuredContent"]["capabilities"]["actions"][2]
            .get("approvalRequired")
            .is_none()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn personal_browser_reads_wait_for_once_session_or_no() {
    for action in ["history.read", "bookmarks.read"] {
        for decision in ["approve", "approve_for_session", "deny"] {
            let core = core(&format!("browser-read-{action}-{decision}")).await;
            let seen = test_support::answering(&core.shared, |tool, args| {
                if tool == "chrome_status" {
                    output_tests::status(args)
                } else {
                    output_tests::mcp(
                        json!({"status":"completed","action":args["action"],"data":{"items":[]}}),
                    )
                }
            });
            let args = json!({"sessionId":"t","action":action,"args":{"limit":20}});
            let shared = core.shared.clone();
            let input = args.clone();
            let task = tokio::spawn(async move {
                crate::agent_tools::prepare(
                    &shared,
                    "s",
                    "s",
                    "call",
                    "chrome_execute",
                    &mut input.clone(),
                )
                .await
            });
            let id = wait_for_card(&core.shared, &task).await;
            assert!(!task.is_finished(), "{action} must wait for a card");
            assert!(
                seen.lock().unwrap().is_empty(),
                "no helper call before consent"
            );
            let cards = tool_calls::open(&core.shared);
            assert_eq!(cards.len(), 1);
            let id = &id;
            let ask = core.shared.asks.get(&format!("callback:{id}")).unwrap();
            assert_eq!(
                ask.offers,
                vec![
                    vw_ask::Choice::Once,
                    vw_ask::Choice::Session,
                    vw_ask::Choice::No
                ]
            );
            tool_calls::answer_by(
                &core.shared,
                Origin::Ui,
                "s",
                id,
                &json!({"decision":{"type":decision}}),
                vw_ask::AnsweredBy::Person,
            )
            .unwrap();
            let result = task.await.unwrap();
            if decision == "deny" {
                assert_eq!(
                    result.err().expect("the read was declined"),
                    "Not run: the user declined this. Do not ask again for this in this task unless the user brings it up."
                );
                assert!(seen.lock().unwrap().is_empty());
            } else {
                result.unwrap().expect("prepared read").await.unwrap();
                let calls = seen.lock().unwrap().clone();
                assert_read_capability(&core.shared, &calls, action);
                if decision == "approve_for_session" {
                    assert!(
                        tokio::time::timeout(
                            std::time::Duration::from_secs(1),
                            prepare(core.shared.clone(), args.clone())
                        )
                        .await
                        .unwrap()
                        .is_some()
                    );
                    let changed = json!({"sessionId":"t","action":action,"args":{"limit":80}});
                    assert!(
                        tokio::time::timeout(
                            std::time::Duration::from_millis(100),
                            prepare(core.shared.clone(), changed)
                        )
                        .await
                        .is_err(),
                        "session consent stays within exact arguments"
                    );
                }
            }
            core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn consent_cannot_survive_a_planning_mode_or_authorize_a_direct_call() {
    let core = core("browser-read-mode-change").await;
    let seen = test_support::answering(&core.shared, |_, _| panic!("no helper call"));
    let args = json!({"sessionId":"t","action":"history.read","args":{}});
    assert!(
        super::gateway::call(&core.shared, "s", "chrome_execute", args.clone())
            .await
            .is_err()
    );
    let task = tokio::spawn(prepare(core.shared.clone(), args));
    let id = wait_for_card(&core.shared, &task).await;
    let id = &id;
    tool_calls::answer_by(
        &core.shared,
        Origin::Ui,
        "s",
        id,
        &json!({"decision":{"type":"approve"}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
    let execution = task.await.unwrap().unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"explore"}),
    );
    assert!(execution.await.is_err());
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn edits_and_full_access_keep_their_existing_read_policy() {
    let core = core("browser-read-other-modes").await;
    for mode in ["accept-edits", "smart-approve", "auto-approve"] {
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        for action in ["history.read", "bookmarks.read"] {
            let args = json!({"sessionId":"t","action":action,"args":{}});
            assert!(super::check::authorize(&core.shared, "s", "chrome_execute", &args).is_ok());
            assert!(prepare(core.shared.clone(), args).await.is_none());
            assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
        }
    }
    core.shutdown().await;
}

fn assert_read_capability(shared: &crate::core::Shared, calls: &[(String, Value)], action: &str) {
    let sent = &calls
        .iter()
        .find(|(tool, _)| tool == "chrome_execute")
        .unwrap()
        .1;
    let caps = sent["_approvedCapabilities"].as_array().unwrap();
    assert!(caps.contains(&json!(action)));
    assert!(!caps.contains(&json!("bookmarks.edit")));
    assert_eq!(super::check::mode(shared, "s").as_deref(), Some("ask"));
}

async fn wait_for_card<T>(
    shared: &crate::core::Shared,
    task: &tokio::task::JoinHandle<T>,
) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(card) = tool_calls::open(shared).first() {
                return card.callback_id.clone();
            }
            assert!(!task.is_finished(), "the read must wait for a card");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the read must open its card")
}
