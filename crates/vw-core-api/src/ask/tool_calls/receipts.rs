//! Single-use decisions issued by the authenticated hook after permission is settled.
//! Binds owner, source/call, tool and input; the tool consumes the prepared action once.
use super::{Calls, PoisonError, Shared, Value, json, names};
use std::sync::Arc;

pub(crate) type Execution =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;
pub(super) struct Receipt {
    key: String,
    owner: String,
    source: String,
    call: String,
    tool: String,
    args: Value,
    prepared_args: Value,
    execution: Option<Execution>,
    stop: u64,
}
fn clean(args: &Value) -> Result<Value, String> {
    let mut args = args
        .as_object()
        .ok_or("Tool input must be an object.")?
        .clone();
    args.remove(names::RECEIPT_FIELD);
    let mut args = Value::Object(args);
    whole_numbers(&mut args);
    Ok(args)
}
/// One spelling per JSON number: a whole number written as a float (`1793404800000.0`, as a JavaScript runtime may
/// send `Date.now()`) becomes the integer the other copy of the same input has, so the comparison sees one value.
/// Only exact whole numbers within 2^53 change; any other number keeps its spelling.
fn whole_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.is_f64() => {
            const EXACT: f64 = 9_007_199_254_740_992.0;
            if let Some(float) = number
                .as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() <= EXACT)
            {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "a whole number within 2^53 is exact"
                )]
                let whole = float as i64;
                *value = json!(whole);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(whole_numbers),
        Value::Object(fields) => fields.values_mut().for_each(whole_numbers),
        _ => {}
    }
}

/// Only the hook credential reaches this route; model arguments cannot supply the source or call.
pub(crate) async fn issue(shared: &Arc<Shared>, payload: &Value) -> Result<Value, String> {
    let field = |key: &str| {
        payload[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| format!("Missing or invalid {key}."))
    };
    let source = field("session_id")?;
    let call = field("tool_call_id")?;
    let owner = payload["parent_session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(source);
    let tool = field("tool_name")?
        .strip_prefix("mcp_vw.")
        .ok_or("Not a Vibe Work tool.")?;
    let mut args = clean(&payload["tool_input"])?;
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or("Tools are unavailable.")?;
    if !tools.table.has_session(owner) {
        return Err("The originating chat is not registered.".into());
    }
    let stop = crate::stops::generation(shared, owner);
    let mut reservation = None;
    let mut replay = None;
    match super::browser_requests::admit(shared, owner, source, call, tool, &args)? {
        Some(super::browser_requests::Admission::New(guard)) => reservation = Some(guard),
        Some(super::browser_requests::Admission::Issued(key)) => {
            args[names::RECEIPT_FIELD] = json!(key);
            return Ok(args);
        }
        Some(super::browser_requests::Admission::Finished(answer)) => replay = Some(answer),
        None => {}
    }
    // Resolve a restart before opening its card and retain that exact launch in the one-use receipt.
    let restart = (tool == names::PROCESS_RESTART)
        .then(|| crate::agent_tools::restart_launch(shared, tools, owner, &args))
        .transpose()?;
    let mut prepared_args = args.clone();
    if let Some(launch) = &restart {
        prepared_args["command"] = json!(launch.command);
        prepared_args["cwd"] = json!(launch.cwd);
        prepared_args["port"] = json!(launch.port);
    }
    // The tool's own refusal (a vault Deny, a login-fill Cancel, a missing program) answers the agent as the
    // tool's result: inside run_typescript, Vibe shows a hook denial only as a generic skip, and the agent
    // retried the call. A No in a Vibe Work prompt asked here (the document card, a Chrome card, the vault window)
    // is such a refusal too: the hook answers with a decision whose tool call returns `vw_ask::DECLINED` and runs
    // nothing, and the turn goes on. A No that stopped the turn stays a denial with its own text.
    // A finished browser request replays its retained answer: no second card, no repeated input.
    let (prepared, shown) = if let Some(answer) = replay {
        let replayed = Box::pin(std::future::ready(super::browser_requests::replay(answer)));
        (Ok(Some(replayed as Execution)), false)
    } else {
        crate::approval_notice::observe(async {
            super::permission(shared, owner, source, call, tool, &prepared_args).await?;
            crate::agent_tools::prepare(shared, owner, source, call, tool, &mut prepared_args).await
        })
        .await
    };
    let execution = match prepared {
        Ok(execution) => execution,
        Err(refusal) if crate::stops::generation(shared, owner) != stop => {
            if let Some(guard) = reservation.take() {
                guard.finish(&Err(refusal.clone()));
            }
            return Err(refusal);
        }
        Err(refusal) => Some(Box::pin(std::future::ready(Err(refusal))) as Execution),
    };
    if shown {
        tools.one_ask.note_allowed(&[source, owner], call);
    }
    if let Some(launch) = restart {
        prepared_args["_vwRestart"] = json!(launch);
    }
    if crate::stops::generation(shared, owner) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    let key = vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?;
    save(
        shared,
        Receipt {
            key: key.clone(),
            owner: owner.into(),
            source: source.into(),
            call: call.into(),
            tool: tool.into(),
            args: args.clone(),
            prepared_args,
            execution,
            stop,
        },
    )?;
    if let Some(guard) = reservation {
        guard.issued(key.clone());
    }
    #[cfg(feature = "video")]
    if tool == "video_frames" {
        // Vibe's effect entry carries this receipt, not the hook's call id: the saved frames are found by it.
        tools.video_history.bind_receipt(source, call, &key);
    }
    args[names::RECEIPT_FIELD] = json!(key);
    Ok(args)
}
fn save(shared: &Shared, receipt: Receipt) -> Result<(), String> {
    let calls = super::calls(shared).ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?;
    let mut pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    pending.retain(|r| crate::stops::generation(shared, &r.owner) == r.stop);
    if pending.len() >= 1000 {
        return Err("Too many tool calls are in progress.".into());
    }
    pending.push_back(receipt);
    Ok(())
}

fn index(
    pending: &std::collections::VecDeque<Receipt>,
    owner: &str,
    tool: &str,
    args: &Value,
) -> Result<usize, String> {
    let key = args[names::RECEIPT_FIELD]
        .as_str()
        .ok_or("This tool needs its pre-tool hook decision.")?;
    let index = pending
        .iter()
        .position(|r| r.key == key)
        .ok_or("Unknown or already used tool decision.")?;
    let receipt = &pending[index];
    if receipt.owner != owner || receipt.tool != tool || receipt.args != clean(args)? {
        return Err("Tool decision does not match this call.".into());
    }
    Ok(index)
}
pub(super) fn verify(
    calls: &Calls,
    owner: &str,
    source: &str,
    callback_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    let pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let receipt = &pending[index(&pending, owner, tool, args)?];
    if receipt.source != source
        || !crate::one_ask::callback_matches_call(callback_id, &receipt.call)
    {
        return Err("This callback does not match its originating Vibe call.".into());
    }
    Ok(())
}
pub(crate) struct Prepared {
    pub(crate) execution: Option<Execution>,
    pub(crate) approved: bool,
}

pub(crate) fn take(
    shared: &Arc<Shared>,
    owner: &str,
    tool: &str,
    args: &mut Value,
) -> Result<Prepared, String> {
    let calls = super::calls(shared).ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?;
    let mut pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let index = index(&pending, owner, tool, args)?;
    let receipt = pending.remove(index).ok_or("Tool decision already used.")?;
    if crate::stops::generation(shared, owner) != receipt.stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    *args = receipt.prepared_args.clone();
    let approved = crate::one_ask::of(shared)
        .is_some_and(|ledger| ledger.user_allowed(&receipt.source, &receipt.call));
    if super::browser_requests::tracked(tool) {
        return browser_execution(shared, receipt).map(|execution| Prepared {
            execution: Some(execution),
            approved,
        });
    }
    Ok(Prepared {
        execution: receipt.execution,
        approved,
    })
}

fn browser_execution(shared: &Arc<Shared>, receipt: Receipt) -> Result<Execution, String> {
    let tools = shared
        .agent_tools
        .as_ref()
        .ok_or(vw_vocabulary::texts::AGENT_TOOLS_UNAVAILABLE)?
        .clone();
    let guard = tools.tool_asks.browser_requests.running(
        (
            receipt.owner.clone(),
            receipt.source.clone(),
            receipt.call.clone(),
        ),
        &receipt.key,
    )?;
    let shared = Arc::downgrade(shared);
    Ok(Box::pin(async move {
        let shared = shared.upgrade().ok_or("Vibe Work closed.")?;
        // Cached receipts never preserve a mode that has since lost site-secret access.
        super::browser_requests::check_full(&shared, &receipt.owner, &receipt.tool, &receipt.args)?;
        let answer = match receipt.execution {
            Some(execution) => execution.await,
            None => {
                crate::agent_tools::call_prepared(
                    &shared,
                    &tools,
                    &receipt.owner,
                    &receipt.tool,
                    receipt.prepared_args,
                )
                .await
            }
        };
        if let Some(guard) = guard {
            guard.finish(&answer);
        }
        answer
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_is_bound_to_owner_source_call_tool_and_input() {
        let calls = Calls::default();
        calls.transports.lock().unwrap().push_back(Receipt {
            key: "decision".into(),
            owner: "parent".into(),
            source: "child".into(),
            call: "call-1".into(),
            tool: "process_start".into(),
            args: json!({"command":"echo once"}),
            prepared_args: Value::Null,
            execution: None,
            stop: 0,
        });
        let args = json!({"command":"echo once","_vwReceipt":"decision"});
        assert!(
            verify(
                &calls,
                "parent",
                "child",
                "approval-call-1",
                "process_start",
                &args
            )
            .is_ok()
        );
        assert!(
            verify(
                &calls,
                "parent",
                "child",
                "approval-call-1-1a2b3c4d",
                "process_start",
                &args
            )
            .is_ok()
        );
        for (owner, source, call, tool, input) in [
            (
                "other",
                "child",
                "approval-call-1",
                "process_start",
                args.clone(),
            ),
            (
                "parent",
                "sibling",
                "approval-call-1",
                "process_start",
                args.clone(),
            ),
            (
                "parent",
                "child",
                "approval-call-2",
                "process_start",
                args.clone(),
            ),
            (
                "parent",
                "child",
                "approval-call-1",
                "process_stop",
                args.clone(),
            ),
            (
                "parent",
                "child",
                "approval-call-1",
                "process_start",
                json!({"command":"echo twice","_vwReceipt":"decision"}),
            ),
        ] {
            assert!(verify(&calls, owner, source, call, tool, &input).is_err());
        }
    }

    /// Seen with Mistral: `history.read {startTime: dayStart, endTime: Date.now()}` was denied "Tool
    /// decision does not match this call." The hook's copy and the tool call's copy of the same arguments can spell a
    /// whole number beyond 32 bits differently (1793404800000 and 1793404800000.0); they are the same JSON number.
    /// PREDICT (before the change): the float spelling of the hook's integer is refused as a mismatch.
    #[test]
    fn the_same_number_in_either_spelling_matches_and_a_different_number_does_not() {
        let calls = Calls::default();
        let decided: Value =
            serde_json::from_str(r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000,"limit":100}}]}"#)
                .unwrap();
        calls.transports.lock().unwrap().push_back(Receipt {
            key: "decision".into(),
            owner: "chat".into(),
            source: "chat".into(),
            call: "call-1".into(),
            tool: "chrome_execute".into(),
            args: clean(&decided).unwrap(),
            prepared_args: Value::Null,
            execution: None,
            stop: 0,
        });
        let called = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
        let same = called(
            r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000.0,"limit":100.0}}],"_vwReceipt":"decision"}"#,
        );
        assert!(
            verify(
                &calls,
                "chat",
                "chat",
                "approval-call-1",
                "chrome_execute",
                &same
            )
            .is_ok()
        );
        for other in [
            r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800001.0,"limit":100}}],"_vwReceipt":"decision"}"#,
            r#"{"steps":[{"action":"history.read","args":{"startTime":1793404800000.5,"limit":100}}],"_vwReceipt":"decision"}"#,
            r#"{"steps":[{"action":"history.read","args":{"startTime":"1793404800000","limit":100}}],"_vwReceipt":"decision"}"#,
        ] {
            assert!(
                verify(
                    &calls,
                    "chat",
                    "chat",
                    "approval-call-1",
                    "chrome_execute",
                    &called(other)
                )
                .is_err(),
                "{other}"
            );
        }
    }
}

#[cfg(test)]
mod decline_tests {
    use super::*;
    #[tokio::test]
    async fn denying_a_tool_card_returns_the_decline_as_the_tool_result() {
        let f = crate::feature_test_core::Fixture::new().await;
        f.load("s", &f.root.join("project"), &[], false);
        let shared = &f.core.shared;
        shared
            .agent_tools
            .as_ref()
            .unwrap()
            .table
            .for_session("s")
            .unwrap();
        let owner = shared.clone();
        let task = tokio::spawn(async move {
            issue(
                &owner,
                &json!({"session_id":"s", "tool_call_id":"read-1",
            "tool_name":"mcp_vw.search", "tool_input":{"query":"needle"}}),
            )
            .await
        });
        let ask = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(ask) = f.core.asks().into_iter().next() {
                    break ask;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        super::super::answer_by(
            shared,
            crate::Origin::Ui,
            "s",
            ask.id.strip_prefix("callback:").unwrap(),
            &json!({"decision":{"type":"deny"}}),
            vw_ask::AnsweredBy::Person,
        )
        .unwrap();
        let decision = task.await.unwrap();
        assert!(
            decision.is_ok(),
            "the hook must retain a declined tool result: {decision:?}"
        );
        let mut args = decision.unwrap();
        let result = take(shared, "s", "search", &mut args)
            .unwrap()
            .execution
            .expect("held refusal")
            .await;
        assert!(result.unwrap_err().contains("the user declined"));
        assert!(take(shared, "s", "search", &mut args).is_err());
        f.finish().await;
    }
}
