//! Bounded ordered built-in actions retain each ordinary permission check and actual receipt.
//! Called by the authenticated hook; stops before later steps after refusal, unmet outcome or budget exhaustion.
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::{future::Future, sync::Arc, time::Duration};
use vw_agent_tools::wire::tools as names;

pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: &Value,
) -> Result<tool_calls::receipts::Execution, String> {
    vw_agent_tools::browser_input::validate(names::BROWSER_ACT, args)?;
    let steps = args["steps"].as_array().ok_or("Missing batch steps.")?;
    let generation = crate::stops::generation(shared, owner);
    // Person waits remain in the pre-tool hook, outside the MCP action's timeout.
    let result = run(steps, |_, step| async move {
        if crate::stops::generation(shared, owner) != generation {
            return (Err(crate::stops::APPROVAL_ENDED.into()), Duration::ZERO);
        }
        let mut step = step.clone();
        let decision =
            tool_calls::permission(shared, owner, source, call, names::BROWSER_ACT, &step).await;
        let prepared = match decision {
            Ok(()) => {
                super::prepare(shared, owner, source, call, names::BROWSER_ACT, &mut step).await
            }
            Err(reason) => return (Err(reason), Duration::ZERO),
        };
        if crate::stops::generation(shared, owner) != generation {
            return (Err(crate::stops::APPROVAL_ENDED.into()), Duration::ZERO);
        }
        let started = tokio::time::Instant::now();
        let result = match prepared {
            Ok(Some(action)) => action.await,
            Ok(None) => super::browser_tools::call(shared, owner, names::BROWSER_ACT, step).await,
            Err(reason) => Err(reason),
        };
        (result, started.elapsed())
    })
    .await;
    Ok(Box::pin(std::future::ready(Ok(result))))
}

fn met(value: &Value) -> bool {
    [value, &value["result"]].into_iter().all(|v| {
        v["met"] != false
            && v["timedOut"] != true
            && !matches!(
                v["status"].as_str(),
                Some("unknown" | "failed" | "refused" | "stopped" | "unmet")
            )
    })
}

async fn run<'a, F, Fut>(steps: &'a [Value], mut execute: F) -> Value
where
    F: FnMut(usize, &'a Value) -> Fut,
    Fut: Future<Output = (Result<Value, String>, Duration)>,
{
    let mut receipts = Vec::new();
    let mut bytes = 400_000usize;
    let mut spent = Duration::ZERO;
    let mut reason = None;
    for (index, step) in steps.iter().enumerate() {
        if spent >= Duration::from_secs(25) {
            reason = Some(
                "Batch execution budget reached; the next step was not dispatched. Continue with the remaining steps after checking these receipts.",
            );
            break;
        }
        // Never drop an in-flight native input to enforce the aggregate budget. Each action keeps its own deadline.
        let (result, elapsed) = execute(index, step).await;
        spent += elapsed;
        let (receipt, continue_after) = match result {
            Ok(value) => {
                let continue_after = met(&value);
                let size = serde_json::to_vec(&value).map_or(usize::MAX, |v| v.len());
                if size > bytes {
                    reason = Some(
                        "Batch output budget reached; this step finished but its content was omitted. Read its current state before continuing; do not repeat input to recover output.",
                    );
                    (
                        json!({"step":index+1,"resultOmitted":true,"conditionMet":continue_after}),
                        false,
                    )
                } else {
                    bytes -= size;
                    (json!({"step":index+1,"result":value}), continue_after)
                }
            }
            Err(error) => (json!({"step":index+1,"error":error}), false),
        };
        receipts.push(receipt);
        if !continue_after {
            reason.get_or_insert("A step was refused or did not establish its requested outcome. Check its receipt before continuing with later steps.");
            break;
        }
    }
    let mut answer =
        json!({"status":if reason.is_some(){"stopped"}else{"completed"},"steps":receipts});
    if let Some(reason) = reason {
        answer["reason"] = json!(reason);
        if receipts.len() < steps.len() {
            answer["nextStep"] = json!(receipts.len() + 1);
        }
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unmet_unknown_and_output_budget_stop_without_replaying() {
        for result in [
            json!({"result":{"met":false}}),
            json!({"status":"unknown"}),
            json!({"text":"x".repeat(400_001)}),
        ] {
            let mut calls = 0;
            let answer = run(&[json!({}), json!({})], |_, _| {
                calls += 1;
                std::future::ready((Ok(result.clone()), Duration::ZERO))
            })
            .await;
            assert_eq!(calls, 1);
            assert_eq!(answer["status"], "stopped");
            assert_eq!(answer["nextStep"], 2);
        }
    }
    #[tokio::test]
    async fn execution_budget_retains_the_finished_step_and_stops_before_next() {
        let mut calls = 0;
        let answer = run(&[json!({}), json!({})], |_, _| {
            calls += 1;
            std::future::ready((Ok(json!({"actual":42})), Duration::from_secs(26)))
        })
        .await;
        assert_eq!(calls, 1);
        assert_eq!(answer["steps"][0]["result"]["actual"], 42);
        assert_eq!(answer["nextStep"], 2);
    }
}
