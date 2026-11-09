//! Browser writes use tiered tool consent; ordinary reads run and secret reads require Full access.
use super::{output_tests, test_support};
use crate::{Origin, ask::tool_calls};
use serde_json::{Value, json};

pub(super) async fn core(name: &str) -> crate::Core {
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
) -> Result<Option<tool_calls::receipts::Execution>, String> {
    crate::agent_tools::prepare(
        &shared,
        "s",
        "s",
        "call",
        "chrome_execute",
        &mut args.clone(),
    )
    .await
}

#[tokio::test]
async fn personal_browser_reads_run_without_cards() {
    let core = core("browser-personal-reads").await;
    for action in [
        "history.read",
        "bookmarks.read",
        "cookies.metadata",
        "storage.metadata",
    ] {
        let args = json!({"sessionId":"t","action":action,"args":{}});
        assert!(super::check::authorize(&core.shared, "s", "chrome_execute", &args).is_ok());
        assert!(prepare(core.shared.clone(), args).await.unwrap().is_none());
        assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn discovery_names_write_approval_and_keeps_reads_available_in_ask() {
    let core = core("browser-read-discovery").await;
    let mut status = json!({"structuredContent":{"capabilities":{"actions":[{"action":"history.read"},{"action":"bookmarks.read"},{"action":"page.type"},{"action":"raw.execute"},{"action":"tabs.close_user"}]}}});
    super::check::annotate_modes(&core.shared, "s", "chrome_status", &mut status);
    let rows = status["structuredContent"]["capabilities"]["actions"]
        .as_array()
        .unwrap();
    for row in rows {
        let asks = matches!(row["action"].as_str(), Some("page.type" | "raw.execute"));
        assert_eq!(row["approvalRequired"] == true, asks, "{row}");
        assert_eq!(row["allowedInThisMode"] == true, !asks, "{row}");
        assert!(row.get("needs").is_none(), "{row}");
    }
    core.shutdown().await;
}

#[tokio::test]
async fn browser_writes_wait_for_once_session_or_no_before_any_helper_effect() {
    for action in ["page.type", "raw.execute"] {
        for decision in ["approve", "approve_for_session", "deny"] {
            let core = core(&format!("browser-write-{action}-{decision}")).await;
            let seen = test_support::answering(&core.shared, |tool, args| {
                if tool == "chrome_status" {
                    output_tests::status(args)
                } else {
                    output_tests::mcp(
                        json!({"status":"completed","action":args["action"],"data":{"changed":true}}),
                    )
                }
            });
            let args = json!({"sessionId":"t","action":action,"args":{"text":"first"}});
            let task = tokio::spawn(prepare(core.shared.clone(), args.clone()));
            let id = wait_for_card(&core.shared, &task).await;
            assert!(seen.lock().unwrap().is_empty());
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
                &id,
                &json!({"decision":{"type":decision}}),
                vw_ask::AnsweredBy::Person,
            )
            .unwrap();
            let result = task.await.unwrap();
            if decision == "deny" {
                assert_eq!(result.err().unwrap(), vw_ask::DECLINED);
                assert!(seen.lock().unwrap().is_empty());
            } else {
                result.unwrap().unwrap().await.unwrap();
                let calls = seen.lock().unwrap().clone();
                let call = &calls
                    .iter()
                    .find(|(tool, _)| tool == "chrome_execute")
                    .unwrap()
                    .1;
                let caps = call["_approvedCapabilities"].as_array().unwrap();
                assert!(caps.contains(&json!("page.edit")));
                assert!(caps.contains(&json!("page.commit")));
                assert_eq!(
                    caps.contains(&json!("raw.execute")),
                    action == "raw.execute"
                );
                drop(calls);
                if decision == "approve_for_session" {
                    let changed = json!({"sessionId":"t","action":"files.upload","args":{"paths":["different.txt"]}});
                    assert!(
                        tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            prepare(core.shared.clone(), changed)
                        )
                        .await
                        .unwrap()
                        .unwrap()
                        .is_some()
                    );
                }
            }
            core.shutdown().await;
        }
    }
}

#[tokio::test]
async fn write_consent_cannot_survive_a_planning_mode_or_authorize_a_direct_call() {
    let core = core("browser-write-mode-change").await;
    let seen = test_support::answering(&core.shared, |_, _| panic!("no helper call"));
    let args = json!({"sessionId":"t","action":"raw.execute","args":{}});
    assert!(
        super::gateway::call(&core.shared, "s", "chrome_execute", args.clone())
            .await
            .is_err()
    );
    let task = tokio::spawn(prepare(core.shared.clone(), args));
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
    let execution = task.await.unwrap().unwrap().unwrap();
    core.shared.access.observe_request(
        "session/agent/update",
        "s",
        &json!({"agentName":"explore"}),
    );
    assert!(execution.await.is_err());
    assert!(seen.lock().unwrap().is_empty());
    core.shutdown().await;
}

pub(super) async fn wait_for_card<T>(
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
