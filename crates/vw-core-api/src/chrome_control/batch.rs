//! Bounded sequential browser work; each step enters the ordinary call-time gateway.
use super::owner;
use serde_json::{Value, json};
use std::future::Future;
use std::time::Duration;

pub(crate) fn steps(args: &Value) -> Result<Vec<Value>, String> {
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

pub(super) async fn run<F, Fut, C>(
    steps: Vec<Value>,
    mut execute: F,
    cancel: C,
    clock: super::batch_wait::Clock,
) -> Value
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
    C: FnOnce(),
{
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut receipts = Vec::new();
    let mut remaining_bytes = 400_000usize;
    let mut images = Vec::new();
    let mut status = "completed";
    let mut cancel = Some(cancel);
    let mut longest_step = Duration::ZERO;
    let mut next_step = None;
    for (index, mut step) in steps.into_iter().enumerate() {
        // Reserve observation time before admitting another input near the batch deadline.
        // A genuine timeout remains unknown; a step stopped here has never been dispatched.
        let reserve = Duration::from_secs(5).max(longest_step.saturating_mul(2));
        if index > 0 && deadline.saturating_duration_since(tokio::time::Instant::now()) <= reserve {
            status = "stopped";
            next_step = Some(index + 1);
            break;
        }
        if step["action"] == "capture.screenshot" || step["action"] == "capture.zoom" {
            step["args"]["saveTo"] = json!("file");
        }
        let started = tokio::time::Instant::now();
        let (result, waited) = clock.until(deadline, execute(step)).await;
        deadline += waited;
        let result = match result {
            Ok(Ok(result)) => result["vwMcpResult"].clone(),
            Ok(Err(reason)) => owner::failure(&reason, "REFUSED"),
            Err(()) => {
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
        longest_step = longest_step.max(started.elapsed().saturating_sub(waited));
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
    let mut body = json!({"status":status,"steps":receipts});
    if let Some(next) = next_step {
        body["nextStep"] = json!(next);
        body["reason"] = json!("Batch time budget: the next step was not dispatched.");
    }
    let mut content = vec![json!({"type":"text","text":body.to_string()})];
    content.extend(images);
    let mut result = json!({"isError":false,"structuredContent":body,"content":content});
    // Modern observations must reach the scoped finisher intact so its outer buffer owns any omission.
    if !super::output_cache::observation(&result) {
        super::output::guard(&mut result);
    }
    json!({"vwMcpResult":result})
}

/// Settle consent before running any step. Prepared actions still recheck the live mode.
pub(crate) async fn prepare(
    shared: &std::sync::Arc<crate::core::Shared>,
    chat: &str,
    source: &str,
    call: &str,
    args: Value,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    let browser = super::agent_browser::selected(shared, chat, "chrome_execute", &args);
    super::gateway::validate_public_args(&args, browser)?;
    super::gateway::validate_session(shared, chat, "chrome_execute", &args)?;
    let mode = super::check::mode(shared, chat);
    let current = mode.as_deref().and_then(vw_ask::chrome::Tier::from_mode);
    let (consent, grant_id) = if current.is_some_and(|tier| tier < vw_ask::chrome::Tier::FullAccess)
    {
        let tier = steps(&args)?
            .iter()
            .filter_map(|step| super::mapping::ask_required("chrome_execute", step))
            .max()
            .unwrap_or(vw_ask::chrome::Tier::Ask);
        let consent = super::read_consent::Consent(tier, false);
        for mut step in steps(&args)? {
            step["sessionId"] = args["sessionId"].clone();
            super::check::authorize_read(shared, chat, "chrome_execute", &step, Some(&consent))?;
        }
        let grant_id = if Some(tier) > current {
            crate::ask::tool_calls::approve_grant(shared,chat,source,call,json!({"kind":"approval","vwBrowserTier":tier as u8,
                "effect":{"kind":"tool","toolName":"mcp_vw.chrome_execute","input":args},"requiredPermissions":[]})).await?
        } else {
            None
        };
        (Some(consent), grant_id)
    } else {
        (None, None)
    };
    let mut prepared = prepare_logins(shared, chat, &args, consent, grant_id.as_deref()).await?;
    let shared = std::sync::Arc::downgrade(shared);
    let chat = chat.to_owned();
    let (source, call) = (source.to_owned(), call.to_owned());
    let execution: crate::ask::tool_calls::receipts::Execution = Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed.")?;
        let id = args["sessionId"].clone();
        let clock = super::batch_wait::Clock::default();
        let action_clock = clock.clone();
        let mut answer = run(
            steps(&args)?,
            |mut step| {
                step["sessionId"] = id.clone();
                let prepared = prepared.pop_front().flatten();
                let shared = &shared;
                let chat = &chat;
                let (source, call) = (&source, &call);
                let action_clock = &action_clock;
                let grant_id = grant_id.as_deref();
                async move {
                    dispatch_with_grant(&shared.ask_grants, grant_id, async move {
                        match prepared {
                            Some(execution) => execution.await,
                            None => {
                                if let Some(consent) = consent {
                                    if super::interaction_consent::checks(&step) {
                                        super::interaction_consent::run(
                                            shared,
                                            chat,
                                            source,
                                            call,
                                            step,
                                            Some(consent),
                                            Some(action_clock),
                                        )
                                        .await
                                    } else {
                                        super::read_consent::execute(shared, chat, step, consent)
                                            .await
                                    }
                                } else {
                                    super::gateway::call_one(shared, chat, "chrome_execute", step)
                                        .await
                                }
                            }
                        }
                    })
                    .await
                }
            },
            || {
                shared
                    .chrome_control
                    .owner
                    .control(json!({"action":"cancel","sessionId":id}));
            },
            clock,
        )
        .await;
        drop(prepared);
        if super::output_cache::observation(&answer["vwMcpResult"]) {
            super::output_cache::bound(&shared, &chat, &args, &mut answer["vwMcpResult"]);
        }
        Ok(answer)
    });
    if consent.is_some() {
        Ok(Box::pin(std::future::ready(execution.await)))
    } else {
        Ok(execution)
    }
}

async fn prepare_logins(
    shared: &std::sync::Arc<crate::core::Shared>,
    chat: &str,
    args: &Value,
    consent: Option<super::read_consent::Consent>,
    grant_id: Option<&str>,
) -> Result<std::collections::VecDeque<Option<crate::ask::tool_calls::receipts::Execution>>, String>
{
    let mut prepared = std::collections::VecDeque::new();
    for mut step in steps(args)? {
        step["sessionId"] = args["sessionId"].clone();
        prepared.push_back(
            if ["auth.fill", "auth.basic"].contains(&step["action"].as_str().unwrap_or_default()) {
                Some(super::login::prepare_with_grant(shared, chat, step, consent, grant_id).await?)
            } else {
                None
            },
        );
    }
    Ok(prepared)
}

async fn dispatch_with_grant<F: Future<Output = Result<Value, String>>>(
    grants: &crate::ask::grants::Grants,
    grant_id: Option<&str>,
    work: F,
) -> Result<Value, String> {
    let admission = match grant_id {
        Some(id) => match grants.admission(id) {
            Some(admission) => Some(admission),
            None => return Ok(revoked_receipt()),
        },
        None => None,
    };
    let result = ADMISSION.scope(admission, work).await;
    match result {
        Err(reason) if reason == crate::ask::tool_calls::REVOKED_BROWSER_GRANT => {
            Ok(revoked_receipt())
        }
        other => other,
    }
}

tokio::task_local! {
    static ADMISSION: Option<crate::ask::grants::Admission>;
}

pub(super) fn admission() -> Option<crate::ask::grants::Admission> {
    ADMISSION.try_with(Clone::clone).ok().flatten()
}

/// Check again after gateway waits and immediately before asking the owner to dispatch.
pub(super) fn admit() -> Result<(), String> {
    if admission().is_some_and(|token| !token.allows()) {
        return Err(crate::ask::tool_calls::REVOKED_BROWSER_GRANT.into());
    }
    Ok(())
}

fn revoked_receipt() -> Value {
    let mut receipt = owner::failure(
        crate::ask::tool_calls::REVOKED_BROWSER_GRANT,
        "CONSENT_REVOKED",
    );
    receipt["structuredContent"]["status"] = json!("not_run");
    json!({"vwMcpResult":receipt})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn a_waiting_batch_step_rechecks_revoke_at_final_dispatch() {
        let grants = std::sync::Arc::new(crate::ask::grants::Grants::default());
        let card = json!({"vwToolAsk":true,"detail":{"kind":"approval","vwBrowserTier":2,
            "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":"t","steps":[
                {"action":"auth.fill","args":{"ref":"u2","login":"Demo"}}]}},"requiredPermissions":[]}});
        grants.note_callback("card", "chat", "chat", crate::ask::grants::scope_of(&card));
        let grant = grants
            .keep(grants.claim("card").unwrap(), vw_ask::AnsweredBy::Person, 1)
            .unwrap();
        let (reached, waiting) = tokio::sync::oneshot::channel();
        let (resume, resumed) = tokio::sync::oneshot::channel();
        let dispatched = std::sync::Arc::new(AtomicBool::new(false));
        let sent = dispatched.clone();
        let held = grants.clone();
        let id = grant.id.clone();
        let step = tokio::spawn(async move {
            dispatch_with_grant(&held, Some(&id), async move {
                reached.send(()).unwrap();
                resumed.await.unwrap();
                admit()?;
                sent.store(true, Ordering::SeqCst);
                Ok(json!({"vwMcpResult":{"structuredContent":{"status":"completed"}}}))
            })
            .await
        });
        waiting.await.unwrap();
        assert!(grants.remove(&grant.id));
        resume.send(()).unwrap();
        let answer = step.await.unwrap().unwrap();
        assert!(
            !dispatched.load(Ordering::SeqCst),
            "a revoked waiting step dispatched"
        );
        assert_eq!(
            answer["vwMcpResult"]["structuredContent"]["status"],
            "not_run"
        );
    }

    #[tokio::test]
    async fn revoking_one_chats_batch_grant_keeps_a_second_chats_dispatch() {
        let grants = crate::ask::grants::Grants::default();
        let card = json!({"vwToolAsk":true,"detail":{"kind":"approval","vwBrowserTier":2,
            "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":"t","steps":[
                {"action":"auth.fill","args":{"ref":"u2","login":"Demo"}}]}},"requiredPermissions":[]}});
        let mut ids = Vec::new();
        for chat in ["first", "second"] {
            grants.note_callback(chat, chat, chat, crate::ask::grants::scope_of(&card));
            ids.push(
                grants
                    .keep(grants.claim(chat).unwrap(), vw_ask::AnsweredBy::Person, 1)
                    .unwrap()
                    .id,
            );
        }
        assert!(grants.remove(&ids[0]));
        let answer = dispatch_with_grant(&grants, Some(&ids[1]), async {
            admit()?;
            Ok(json!({"status":"completed"}))
        })
        .await
        .unwrap();
        assert_eq!(answer["status"], "completed");
    }

    /// PREDICT: a batch keeps its prepared login executions after the chat grant is revoked,
    /// so the second saved-login field is sent despite Revoke.
    #[tokio::test]
    async fn revoked_batch_grant_stops_the_next_prepared_fill() {
        for revoke in [true, false] {
            let grants = std::sync::Arc::new(crate::ask::grants::Grants::default());
            let card = json!({"vwToolAsk":true,"detail":{"kind":"approval","vwBrowserTier":2,
                "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":"t","steps":[
                    {"action":"auth.fill","args":{"ref":"u2","login":"Demo"}},
                    {"action":"auth.fill","args":{"ref":"u3","login":"Demo"}},
                    {"action":"page.read","args":{}}]}},
                "requiredPermissions":[]}});
            grants.note_callback("card", "s", "s", crate::ask::grants::scope_of(&card));
            let grant = grants
                .keep(grants.claim("card").unwrap(), vw_ask::AnsweredBy::Person, 1)
                .unwrap();
            grants.note_callback(
                "other-card",
                "other-chat",
                "other-chat",
                crate::ask::grants::scope_of(&card),
            );
            let other_grant = grants
                .keep(
                    grants.claim("other-card").unwrap(),
                    vw_ask::AnsweredBy::Person,
                    1,
                )
                .unwrap();
            let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let result = run(
                vec![json!({"action":"auth.fill","args":{"ref":"u2"}}),
                     json!({"action":"auth.fill","args":{"ref":"u3"}}),
                     json!({"action":"page.read","args":{}})],
                |_| {
                    let id = grant.id.clone();
                    let grants = grants.clone();
                    let sent = sent.clone();
                    async move {
                        let sent_for_execution = sent.clone();
                        let grants_for_execution = grants.clone();
                        let id_for_execution = id.clone();
                        let execution: crate::ask::tool_calls::receipts::Execution = Box::pin(async move {
                            let number = sent_for_execution.fetch_add(1, Ordering::SeqCst) + 1;
                            if revoke && number == 1 {
                                grants_for_execution.remove(&id_for_execution);
                            }
                            Ok(json!({"vwMcpResult":{"structuredContent":{"status":"completed","data":{"filled":true}}}}))
                        });
                        dispatch_with_grant(&grants, Some(&id), execution).await
                    }
                },
                || panic!("not timed out"),
                super::super::batch_wait::Clock::default(),
            ).await;
            assert_eq!(sent.load(Ordering::SeqCst), if revoke { 1 } else { 3 });
            assert!(grants.contains(&other_grant.id));
            let body = &result["vwMcpResult"]["structuredContent"];
            assert_eq!(body["steps"][0]["result"]["status"], "completed");
            assert_eq!(body["status"], if revoke { "stopped" } else { "completed" });
            assert_eq!(
                body["steps"].as_array().unwrap().len(),
                if revoke { 2 } else { 3 }
            );
            assert_eq!(
                body["steps"][1]["result"]["status"],
                if revoke { "not_run" } else { "completed" }
            );
            if revoke {
                assert!(
                    body["steps"][1]["result"]["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("Ask the user")
                );
            }
        }
    }
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
            }, ||panic!("no timeout"), super::super::batch_wait::Clock::default()).await;
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
            super::super::batch_wait::Clock::default(),
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
        }, ||panic!("no timeout"), super::super::batch_wait::Clock::default()).await;
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
    async fn repeated_clicks_stop_before_a_step_without_observation_time() {
        let mut dispatched = 0;
        let result = run(
            vec![json!({"action":"page.click","args":{"ref":"previous"}}); 16],
            |_| {
                dispatched += 1;
                async {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    Ok(json!({"vwMcpResult":{"structuredContent":{"status":"completed","data":{"effect":"page_changed"}}}}))
                }
            },
            || panic!("a predictable batch tail must not fence a dispatched click"),
            super::super::batch_wait::Clock::default(),
        ).await;
        let body = &result["vwMcpResult"]["structuredContent"];
        assert_eq!(body["status"], "stopped");
        assert_eq!(body["steps"].as_array().unwrap().len(), dispatched);
        assert!(
            body["steps"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["result"]["status"] == "completed")
        );
        assert_eq!(body["nextStep"], dispatched + 1);
        assert!(dispatched < 16);
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
            super::super::batch_wait::Clock::default(),
        )
        .await;
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(
            result["vwMcpResult"]["structuredContent"]["steps"][0]["result"]["status"],
            "unknown"
        );
    }
}
