//! Bounded sequential browser work; each step enters the ordinary call-time gateway.
use super::owner;
use serde_json::{Value, json};
use std::future::Future;
use std::time::Duration;

pub(super) fn steps(args: &Value) -> Result<Vec<Value>, String> {
    let object = args.as_object().ok_or("Not run: invalid batch.")?;
    if object
        .keys()
        .any(|key| !["sessionId", "steps"].contains(&key.as_str()))
    {
        return Err("Not run: a batch takes sessionId and steps.".into());
    }
    let steps = args["steps"]
        .as_array()
        .filter(|steps| !steps.is_empty() && steps.len() <= 16)
        .ok_or("Not run: a batch needs 1 to 16 steps.")?;
    for step in steps {
        if step.as_object().is_none_or(|step| {
            step.keys()
                .any(|key| !["action", "args"].contains(&key.as_str()))
        }) || !step["action"].is_string()
            || !step["args"].is_object()
        {
            return Err("Not run: each step takes action and args.".into());
        }
    }
    Ok(steps.clone())
}

pub(super) async fn run<F, Fut, C>(steps: Vec<Value>, mut execute: F, cancel: C) -> Value
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
    C: FnOnce(),
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut receipts = Vec::new();
    let mut remaining_bytes = 400_000usize;
    let mut images = Vec::new();
    let mut status = "completed";
    let mut cancel = Some(cancel);
    for (index, mut step) in steps.into_iter().enumerate() {
        if step["action"] == "capture.screenshot" || step["action"] == "capture.zoom" {
            step["args"]["saveTo"] = json!("file");
        }
        let result = match tokio::time::timeout_at(deadline, execute(step)).await {
            Ok(Ok(result)) => result["vwMcpResult"].clone(),
            Ok(Err(reason)) => owner::failure(&reason, "REFUSED"),
            Err(_) => {
                if let Some(cancel) = cancel.take() {
                    cancel();
                }
                let mut result = owner::failure(
                    "The batch exceeded its time limit. The current action's outcome is unknown.",
                    "ACTION_TIMEOUT",
                );
                result["structuredContent"]["status"] = json!("unknown");
                result
            }
        };
        if let Some(content) = result["content"].as_array() {
            images.extend(
                content
                    .iter()
                    .filter(|item| item["type"] == "image")
                    .cloned(),
            );
        }
        let mut body = result["structuredContent"].clone();
        let bytes = serde_json::to_vec(&body).map_or(usize::MAX, |bytes| bytes.len());
        let omitted = bytes > remaining_bytes;
        if omitted {
            body = json!({"operationId":body["operationId"],"status":body["status"],"resultOmitted":"Batch output limit reached"});
        }
        remaining_bytes = remaining_bytes.saturating_sub(bytes);
        let completed = body["status"] == "completed";
        receipts.push(json!({"step":index+1,"result":body}));
        if !completed || omitted {
            status = "stopped";
            break;
        }
    }
    let body = json!({"status":status,"steps":receipts});
    let mut content = vec![json!({"type":"text","text":body.to_string()})];
    content.extend(images);
    json!({"vwMcpResult":{"isError":false,"structuredContent":body,"content":content}})
}

/// Settle vault decisions before running any step. Prepared writes still recheck the live mode.
pub(crate) async fn prepare(
    shared: &std::sync::Arc<crate::core::Shared>,
    chat: &str,
    args: Value,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    super::gateway::validate_public_args(&args)?;
    super::gateway::validate_session(shared, chat, "chrome_execute", &args)?;
    let mut prepared = std::collections::VecDeque::new();
    for mut step in steps(&args)? {
        step["sessionId"] = args["sessionId"].clone();
        prepared.push_back(
            if ["auth.fill", "auth.basic"].contains(&step["action"].as_str().unwrap_or_default()) {
                Some(super::login::prepare(shared, chat, step).await?)
            } else {
                None
            },
        );
    }
    let shared = std::sync::Arc::downgrade(shared);
    let chat = chat.to_owned();
    Ok(Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed.")?;
        let id = args["sessionId"].clone();
        Ok(run(
            steps(&args)?,
            |mut step| {
                step["sessionId"] = id.clone();
                let prepared = prepared.pop_front().flatten();
                let shared = &shared;
                let chat = &chat;
                async move {
                    match prepared {
                        Some(execution) => execution.await,
                        None => {
                            super::gateway::call_one(shared, chat, "chrome_execute", step).await
                        }
                    }
                }
            },
            || {
                shared
                    .chrome_control
                    .owner
                    .control(json!({"action":"cancel","sessionId":id}));
            },
        )
        .await)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn rejects_empty_oversized_and_mixed_batches() {
        for args in [
            json!({"steps":[]}),
            json!({"steps":vec![json!({"action":"page.read","args":{}});17]}),
            json!({"action":"page.read","steps":[{"action":"page.read","args":{}}]}),
            json!({"steps":[{"action":"page.read","args":{},"_approvedCapabilities":[]}]}),
        ] {
            assert!(steps(&args).is_err());
        }
    }
    #[tokio::test]
    async fn a_refusal_or_unknown_result_stops_later_steps() {
        for stop in ["failed", "unknown"] {
            let mut calls = 0;
            let result=run(vec![json!(1),json!(2),json!(3)], |_| {
                calls+=1;
                std::future::ready(Ok(json!({"vwMcpResult":{"structuredContent":{"status":if calls==1{"completed"}else{stop}}}})))
            }, ||panic!("no timeout")).await;
            assert_eq!(calls, 2);
            assert_eq!(
                result["vwMcpResult"]["structuredContent"]["steps"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        }
    }
    #[tokio::test]
    async fn next_step_uses_a_changed_chat_mode() {
        let core = crate::offline_core::start("chrome-batch-mode").await;
        let shared = &core.shared;
        let _ = shared
            .store
            .load(
                json!({"session":{"id":"s"},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"accept-edits"}),
        );
        let mut calls = 0;
        let result = run(
            vec![
                json!({"action":"page.read","args":{}}),
                json!({"action":"page.type","args":{"text":"x"}}),
            ],
            |step| {
                calls += 1;
                let allowed = super::super::check::authorize(shared, "s", "chrome_execute", &step);
                shared.access.observe_request(
                    "session/agent/update",
                    "s",
                    &json!({"agentName":"ask"}),
                );
                std::future::ready(
                    allowed.map(
                        |_| json!({"vwMcpResult":{"structuredContent":{"status":"completed"}}}),
                    ),
                )
            },
            || panic!("not timed out"),
        )
        .await;
        assert_eq!(calls, 2);
        assert!(
            result["vwMcpResult"]["structuredContent"]["steps"][1]["result"]["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&vw_ask::chrome::Tier::AcceptEdits.required_label())
        );
        assert_eq!(shared.asks.list(), [] as [vw_ask::Ask; 0]);
        core.shutdown().await;
    }

    #[tokio::test]
    async fn output_limit_keeps_receipts_without_running_more_steps() {
        let mut calls = 0;
        let result = run(vec![json!({});3], |_| {
            calls += 1;
            std::future::ready(Ok(json!({"vwMcpResult":{"structuredContent":{"status":"completed","operationId":calls,"data":"x".repeat(250_000)}}})))
        }, ||panic!("no timeout")).await;
        assert_eq!(calls, 2);
        let steps = &result["vwMcpResult"]["structuredContent"]["steps"];
        assert_eq!(steps[1]["result"]["status"], "completed");
        assert_eq!(
            steps[1]["result"]["resultOmitted"],
            "Batch output limit reached"
        );
        assert!(serde_json::to_vec(&result).unwrap().len() < 1_000_000);
    }

    #[tokio::test(start_paused = true)]
    async fn total_deadline_cancels_the_current_step() {
        let cancelled = AtomicBool::new(false);
        let result = run(
            vec![json!(1)],
            |_| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(Value::Null)
            },
            || cancelled.store(true, Ordering::SeqCst),
        )
        .await;
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["steps"][0]["result"]["status"],
            "unknown"
        );
    }
}
