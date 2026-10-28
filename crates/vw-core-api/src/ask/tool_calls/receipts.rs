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
    Ok(Value::Object(args))
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
    super::permission(shared, owner, source, call, tool, &args).await?;
    let mut prepared_args = args.clone();
    // The tool's own refusal (a vault Deny, a login-fill Cancel, a missing program) answers the agent as the
    // tool's result: inside run_typescript, Vibe shows a hook denial only as a generic skip, and the agent
    // retried the call. A No that stopped the turn stays a denial with its own text.
    let execution =
        match crate::agent_tools::prepare(shared, owner, source, call, tool, &mut prepared_args)
            .await
        {
            Ok(execution) => execution,
            Err(refusal) if crate::stops::generation(shared, owner) != stop => return Err(refusal),
            Err(refusal) => Some(Box::pin(std::future::ready(Err(refusal))) as Execution),
        };
    if crate::stops::generation(shared, owner) != stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    let key = vw_agent_tools::credential::new_credential().map_err(|e| e.to_string())?;
    let mut pending = tools
        .tool_asks
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    pending.retain(|r| crate::stops::generation(shared, &r.owner) == r.stop);
    if pending.len() >= 1000 {
        return Err("Too many tool calls are in progress.".into());
    }
    pending.push_back(Receipt {
        key: key.clone(),
        owner: owner.into(),
        source: source.into(),
        call: call.into(),
        tool: tool.into(),
        args: args.clone(),
        prepared_args,
        execution,
        stop,
    });
    args[names::RECEIPT_FIELD] = json!(key);
    Ok(args)
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
    call: &str,
    tool: &str,
    args: &Value,
) -> Result<(), String> {
    let pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let receipt = &pending[index(&pending, owner, tool, args)?];
    if receipt.source != source || receipt.call != call {
        return Err("This callback does not match its originating Vibe call.".into());
    }
    Ok(())
}
pub(crate) fn take(
    shared: &Shared,
    owner: &str,
    tool: &str,
    args: &mut Value,
) -> Result<Option<Execution>, String> {
    let calls = super::calls(shared).ok_or("Tools unavailable.")?;
    let mut pending = calls
        .transports
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let index = index(&pending, owner, tool, args)?;
    let receipt = pending.remove(index).ok_or("Tool decision already used.")?;
    if crate::stops::generation(shared, owner) != receipt.stop {
        return Err(crate::stops::APPROVAL_ENDED.into());
    }
    *args = receipt.prepared_args;
    Ok(receipt.execution)
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
        assert!(verify(&calls, "parent", "child", "call-1", "process_start", &args).is_ok());
        for (owner, source, call, tool, input) in [
            ("other", "child", "call-1", "process_start", args.clone()),
            ("parent", "sibling", "call-1", "process_start", args.clone()),
            ("parent", "child", "call-2", "process_start", args.clone()),
            ("parent", "child", "call-1", "process_stop", args.clone()),
            (
                "parent",
                "child",
                "call-1",
                "process_start",
                json!({"command":"echo twice","_vwReceipt":"decision"}),
            ),
        ] {
            assert!(verify(&calls, owner, source, call, tool, &input).is_err());
        }
    }
}
