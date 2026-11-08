//! Ask-mode consent for personal reads and closing exact tabs. The authenticated hook keeps the prepared call in its receipt.
//! Uses the existing card/grant service; no mode change, helper retry or model instructions.
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;

/// Core-only preflight marker; execution receives it only after consent. Never deserialized from tool input.
pub(super) struct Consent(());

pub(crate) fn personal_read(tool: &str, args: &Value) -> bool {
    tool == "chrome_execute"
        && matches!(
            args["action"].as_str(),
            Some("history.read" | "bookmarks.read")
        )
}

pub(crate) fn card_action(tool: &str, args: &Value) -> bool {
    personal_read(tool, args)
        || tool == "chrome_execute"
            && matches!(
                args["action"].as_str(),
                Some("tabs.close_owned" | "tabs.close_user")
            )
}

pub(crate) fn needed(shared: &Shared, owner: &str, args: &Value) -> bool {
    super::check::mode(shared, owner).as_deref() == Some("ask")
        && card_action("chrome_execute", args)
}

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: Value,
) -> Result<tool_calls::receipts::Execution, String> {
    super::gateway::validate_execute_args(&args)?;
    let (args, reopened) = match super::reopen::resolve(shared, owner, "chrome_execute", args)
        .await?
    {
        super::reopen::Resolved::Ready(args, reopened) => (args, reopened),
        super::reopen::Resolved::Refused(answer) => return Ok(Box::pin(async move { Ok(answer) })),
    };
    // Check availability, chat ownership and the current mode before opening any card.
    super::check::authorize_read(shared, owner, "chrome_execute", &args, Some(&Consent(())))?;
    super::gateway::validate_session(shared, owner, "chrome_execute", &args)?;
    let mut detail = json!({"kind":"approval","effect":{"kind":"tool","toolName":"mcp_vw.chrome_execute","input":args},"requiredPermissions":[]});
    if !personal_read("chrome_execute", &args) {
        let reply = shared
            .chrome_control
            .owner
            .call(
                "chrome_status",
                json!({"_closeTab":{"sessionId":args["sessionId"],"tabId":args["args"]["tabId"]}}),
            )
            .await
            .into_value();
        let tab = &reply["structuredContent"]["tab"];
        if !tab.is_object() || tab["tabId"] != args["args"]["tabId"] {
            return Err("Not run: this tab is unavailable to this chat. Use a tab in this chat window or a window handed to this chat.".into());
        }
        detail["effect"]["display"] = json!({"tab":tab});
    }
    tool_calls::approve(shared, owner, source, call, detail).await?;
    let shared = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = shared
            .upgrade()
            .ok_or("Browser closed. Reopen the session before trying again.")?;
        let mut answer = execute(&shared, &owner, args, Consent(())).await?;
        if let Some(reopened) = reopened {
            super::reopen::note(&mut answer, &reopened);
        }
        Ok(answer)
    }))
}

async fn execute(
    shared: &Arc<Shared>,
    session: &str,
    args: Value,
    consent: Consent,
) -> Result<Value, String> {
    let request = args.clone();
    let mut answer =
        super::gateway::call_one_inner(shared, session, "chrome_execute", args, Some(&consent))
            .await?;
    super::receipt::stamp(
        shared,
        "chrome_execute",
        &request,
        &mut answer["vwMcpResult"],
    );
    super::output::guard(&mut answer["vwMcpResult"]);
    Ok(answer)
}

/// Sends a prepared read through its narrow consent path; all other calls keep their existing route.
pub(super) async fn send(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: Value,
    consent: Option<&Consent>,
) -> Result<super::owner::Answer, String> {
    if consent.is_some() {
        super::escalation::send_read(shared, session, tool, args, consent).await
    } else {
        super::login::send(shared, session, tool, args).await
    }
}
