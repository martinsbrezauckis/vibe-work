//! Helper-discovered sends ask in the hook, and only a proven no-effect operation may be retried.
//! Uses authenticated helper-shaped answers and the real browser card/receipt path.
use super::{
    output_tests,
    read_consent_tests::{core, wait_for_card},
    test_support,
};
use crate::{Origin, ask::tool_calls};
use serde_json::json;

#[tokio::test]
async fn a_known_write_batch_asks_once_before_any_step() {
    let core = core("browser-batch-package").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            output_tests::status(args)
        } else {
            output_tests::mcp(json!({"status":"completed","data":{"action":args["action"]}}))
        }
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        crate::agent_tools::prepare(
            &shared,
            "s",
            "s",
            "batch-write",
            "chrome_execute",
            &mut json!({"sessionId":"t","steps":[
                {"action":"page.type","args":{"text":"example"}},
                {"action":"page.commit","args":{}},
                {"action":"files.upload","args":{"paths":["example.txt"]}}
            ]}),
        )
        .await
    });
    let id = wait_for_card(&core.shared, &task).await;
    assert!(seen.lock().unwrap().is_empty());
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
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["status"],
        "completed"
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        3
    );
    assert_eq!(core.shared.asks.list(), [] as [vw_ask::Ask; 0]);
    core.shutdown().await;
}

#[tokio::test]
async fn a_dynamic_batch_card_keeps_waiting_beyond_the_browser_work_deadline() {
    let core = core("browser-batch-card-clock").await;
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        if args["_approvedCapabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("page.commit"))
        {
            output_tests::mcp(
                json!({"status":"completed","operationId":"sent","data":{"sent":true}}),
            )
        } else {
            output_tests::mcp(json!({"status":"failed","operationId":"inspected",
            "error":{"code":"CAPABILITY_REQUIRED","details":{"capability":"page.commit","noEffect":true}}}))
        }
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        crate::agent_tools::prepare(
            &shared,
            "s",
            "s",
            "batch-clock",
            "chrome_execute",
            &mut json!({"sessionId":"t","steps":[{"action":"page.click","args":{"ref":"button"}}]}),
        )
        .await
    });
    let id = wait_for_card(&core.shared, &task).await;
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(120)).await;
    tokio::task::yield_now().await;
    assert!(
        !task.is_finished(),
        "a person deciding on a card must not consume the batch work budget"
    );
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
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["status"],
        "completed"
    );
    tokio::time::resume();
    core.shutdown().await;
}

#[tokio::test]
async fn a_legacy_capability_failure_asks_without_guessing_that_input_can_be_replayed() {
    let core = core("browser-legacy-send").await;
    let seen = test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            output_tests::status(args)
        } else {
            output_tests::mcp(
                json!({"status":"failed","operationId":"old","error":{"code":"CAPABILITY_REQUIRED"}}),
            )
        }
    });
    let shared = core.shared.clone();
    let task = tokio::spawn(async move {
        crate::agent_tools::prepare(
            &shared,
            "s",
            "s",
            "legacy",
            "chrome_execute",
            &mut json!({"sessionId":"t","action":"page.click","args":{"ref":"button"}}),
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
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["status"],
        "failed"
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(tool, _)| tool == "chrome_execute")
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_click_that_needs_sending_asks_and_retries_only_a_no_effect_operation() {
    for no_effect in [true, false] {
        for decision in ["approve", "deny"] {
            let core = core(&format!("browser-click-send-{no_effect}-{decision}")).await;
            let seen = test_support::answering(&core.shared, move |tool, args| {
                if tool == "chrome_status" {
                    return output_tests::status(args);
                }
                if args["_approvedCapabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("page.commit"))
                {
                    assert_eq!(args["_preflightOperationId"], "inspected-click");
                    output_tests::mcp(
                        json!({"status":"completed","operationId":"sent","data":{"sent":true}}),
                    )
                } else {
                    output_tests::mcp(json!({"status":"failed","operationId":"inspected-click",
                    "error":{"code":"CAPABILITY_REQUIRED","details":{"capability":"page.commit","noEffect":no_effect}}}))
                }
            });
            let shared = core.shared.clone();
            let task = tokio::spawn(async move {
                crate::agent_tools::prepare(
                    &shared,
                    "s",
                    "s",
                    "click-call",
                    "chrome_execute",
                    &mut json!({"sessionId":"t","action":"page.click","args":{"ref":"button"}}),
                )
                .await
            });
            let id = wait_for_card(&core.shared, &task).await;
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
            } else {
                let result = result.unwrap().unwrap().await.unwrap();
                assert_eq!(
                    result["vwMcpResult"]["structuredContent"]["status"],
                    if no_effect { "completed" } else { "failed" }
                );
            }
            assert_eq!(
                seen.lock()
                    .unwrap()
                    .iter()
                    .filter(|(tool, _)| tool == "chrome_execute")
                    .count(),
                if decision == "approve" && no_effect {
                    2
                } else {
                    1
                }
            );
            core.shutdown().await;
        }
    }
}
