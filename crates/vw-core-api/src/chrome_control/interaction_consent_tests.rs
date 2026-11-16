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
        "the user deciding on a card must not consume the batch work budget"
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

/// The detail of one open app card, as the transcript overlay shows it.
fn card_detail(shared: &crate::core::Shared, id: &str) -> serde_json::Value {
    let mut state = json!({"history":[]});
    tool_calls::overlay(shared, "s", &mut state);
    state["history"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .map(|entry| entry["detail"].clone())
        .expect("the open card is in the transcript")
}

fn answer(shared: &std::sync::Arc<crate::core::Shared>, id: &str, decision: &str) {
    tool_calls::answer_by(
        shared,
        Origin::Ui,
        "s",
        id,
        &json!({"decision":{"type":decision}}),
        vw_ask::AnsweredBy::Person,
    )
    .unwrap();
}

/// A helper whose form check sends only with `page.commit` and otherwise refuses before any input.
fn form_gate_helper(
    shared: &crate::core::Shared,
) -> std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> {
    test_support::answering(shared, |tool, args| {
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
            output_tests::mcp(json!({"status":"failed","operationId":"held",
            "error":{"code":"CAPABILITY_REQUIRED","details":{"capability":"page.commit","noEffect":true}}}))
        }
    })
}

/// The `_approvedCapabilities` of each `chrome_execute` the helper received.
fn executed(seen: &std::sync::Mutex<Vec<(String, serde_json::Value)>>) -> Vec<serde_json::Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(tool, _)| tool == "chrome_execute")
        .map(|(_, args)| args["_approvedCapabilities"].clone())
        .collect()
}

type Prepared =
    tokio::task::JoinHandle<Result<Option<crate::ask::tool_calls::receipts::Execution>, String>>;

fn start(core: &crate::Core, mut args: serde_json::Value) -> Prepared {
    args["sessionId"] = json!("t");
    let shared = core.shared.clone();
    tokio::spawn(async move {
        crate::agent_tools::prepare(&shared, "s", "s", "write", "chrome_execute", &mut args).await
    })
}

/// Another open card than `first`, or `None` once the call has finished without one.
async fn next_card(core: &crate::Core, first: &str, task: &Prepared) -> Option<String> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(card) = tool_calls::open(&core.shared)
                .into_iter()
                .find(|card| card.callback_id != first)
            {
                return Some(card.callback_id);
            }
            if task.is_finished() {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

/// PREDICT: in Ask, a write card that did not name a send (page.drag "Drag an element", page.key "Press Enter",
/// page.type without submit) gave the call the whole Accept edits package, `page.commit` included. The helper's own
/// form check then let a POST submit under the pointer or the focused field go out with only that one card: the
/// first helper call already carried `page.commit` and no second card named the send. After the fix the first call
/// lacks it, the helper's no-effect refusal opens a card naming the send, and only that answer adds it.
#[tokio::test]
async fn an_ask_write_card_that_did_not_name_a_send_does_not_approve_one() {
    for (name, input) in [
        (
            "drag",
            json!({"action":"page.drag","args":{"ref":"submit","path":[],"holdMs":1000}}),
        ),
        ("enter", json!({"action":"page.key","args":{"key":"Enter"}})),
        (
            "type",
            json!({"action":"page.type","args":{"ref":"field","text":"x"}}),
        ),
    ] {
        let core = core(&format!("browser-unnamed-send-{name}")).await;
        let seen = form_gate_helper(&core.shared);
        let task = start(&core, input);
        let first = wait_for_card(&core.shared, &task).await;
        assert_eq!(
            card_detail(&core.shared, &first)["vwBrowserAction"],
            json!(null),
            "{name}"
        );
        answer(&core.shared, &first, "approve");
        let Some(second) = next_card(&core, &first, &task).await else {
            panic!(
                "{name}: the send ran under the write card alone; helper capabilities {:?}",
                executed(&seen)
            );
        };
        assert_eq!(
            card_detail(&core.shared, &second)["vwBrowserAction"],
            "page.commit",
            "{name}"
        );
        answer(&core.shared, &second, "approve");
        let result = task.await.unwrap().unwrap().unwrap().await.unwrap();
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{name}"
        );
        let calls = executed(&seen);
        assert_eq!(calls.len(), 2, "{name}: {calls:?}");
        assert!(
            !calls[0].as_array().unwrap().contains(&json!("page.commit")),
            "{name}"
        );
        assert!(
            calls[1].as_array().unwrap().contains(&json!("page.commit")),
            "{name}"
        );
        core.shutdown().await;
    }
}

/// A card whose own words name the send (page.commit "Submit a form", page.type "… and submit it") keeps sending
/// authority with that one card, and Accept edits keeps it without any card.
#[tokio::test]
async fn a_card_naming_the_send_and_accept_edits_keep_sending_authority() {
    for (name, mode, input, card) in [
        (
            "commit",
            "ask",
            json!({"action":"page.commit","args":{"ref":"submit"}}),
            true,
        ),
        (
            "type-submit",
            "ask",
            json!({"action":"page.type","args":{"ref":"field","text":"x","submit":true}}),
            true,
        ),
        (
            "drag-accept-edits",
            "accept-edits",
            json!({"action":"page.drag","args":{"ref":"submit","path":[],"holdMs":1000}}),
            false,
        ),
    ] {
        let core = core(&format!("browser-named-send-{name}")).await;
        core.shared
            .access
            .observe_request("session/agent/update", "s", &json!({"agentName":mode}));
        let seen = form_gate_helper(&core.shared);
        let task = start(&core, input.clone());
        if card {
            let id = wait_for_card(&core.shared, &task).await;
            answer(&core.shared, &id, "approve");
        }
        let result = if let Some(execute) = task.await.unwrap().unwrap() {
            execute.await.unwrap()
        } else {
            let mut args = input;
            args["sessionId"] = json!("t");
            super::gateway::call(&core.shared, "s", "chrome_execute", args)
                .await
                .unwrap()
        };
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["status"], "completed",
            "{name}"
        );
        assert_eq!(
            tool_calls::open(&core.shared),
            [] as [crate::CallbackInfo; 0],
            "{name}"
        );
        let calls = executed(&seen);
        assert_eq!(calls.len(), 1, "{name}: {calls:?}");
        core.shutdown().await;
    }
}

/// "Allow for this chat" on a write card lists the whole package, form sends included: a send that the helper
/// then discovers is covered by that grant and retried without another card.
#[tokio::test]
async fn a_chat_grant_for_the_write_package_covers_a_discovered_send_without_another_card() {
    let core = core("browser-unnamed-send-grant").await;
    let seen = form_gate_helper(&core.shared);
    let task = start(
        &core,
        json!({"action":"page.drag","args":{"ref":"submit","path":[],"holdMs":1000}}),
    );
    let id = wait_for_card(&core.shared, &task).await;
    answer(&core.shared, &id, "approve_for_session");
    assert_eq!(next_card(&core, &id, &task).await, None);
    let result = task.await.unwrap().unwrap().unwrap().await.unwrap();
    assert_eq!(
        result["vwMcpResult"]["structuredContent"]["status"],
        "completed"
    );
    assert_eq!(executed(&seen).len(), 2);
    core.shutdown().await;
}
