//! Consent for a browser tool's tiered write package. The authenticated hook keeps the prepared call in its receipt.
//! Uses the existing card/grant service; no mode change, helper retry or model instructions.
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;

/// Core-only preflight marker; execution receives it only after consent. Never deserialized from tool input.
#[derive(Clone, Copy)]
pub(super) struct Consent(pub(super) vw_ask::chrome::Tier);

pub(crate) fn personal_read(tool: &str, args: &Value) -> bool {
    tool == "chrome_execute"
        && matches!(
            args["action"].as_str(),
            Some("history.read" | "bookmarks.read")
        )
}

pub(crate) fn needed(shared: &Shared, owner: &str, args: &Value) -> bool {
    super::mapping::approval_tier(
        super::check::mode(shared, owner).as_deref(),
        "chrome_execute",
        args,
    )
    .is_some()
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
    let consent =
        Consent(super::mapping::ask_required("chrome_execute", &args).ok_or(
            "Not run: unknown Chrome action. Use chrome_status to find supported actions.",
        )?);
    super::check::authorize_read(shared, owner, "chrome_execute", &args, Some(&consent))?;
    super::gateway::validate_session(shared, owner, "chrome_execute", &args)?;
    let detail = json!({"kind":"approval","vwBrowserTier":consent.0 as u8,"effect":{"kind":"tool","toolName":"mcp_vw.chrome_execute","input":args},"requiredPermissions":[]});
    tool_calls::approve(shared, owner, source, call, detail).await?;
    if matches!(args["action"].as_str(), Some("auth.fill" | "auth.basic")) {
        return super::login::prepare_consented(shared, owner, args, Some(consent)).await;
    }
    if super::interaction_consent::checks(&args) {
        return super::interaction_consent::prepare_consented(
            shared,
            owner,
            source,
            call,
            args,
            Some(consent),
        )
        .await;
    }
    let shared = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = shared
            .upgrade()
            .ok_or("Browser closed. Reopen the session before trying again.")?;
        let mut answer = execute(&shared, &owner, args, consent).await?;
        if let Some(reopened) = reopened {
            super::reopen::note(&mut answer, &reopened);
        }
        Ok(answer)
    }))
}

pub(super) async fn execute(
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

/// Sends a prepared browser package through its checked consent path; other calls keep their existing route.
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
