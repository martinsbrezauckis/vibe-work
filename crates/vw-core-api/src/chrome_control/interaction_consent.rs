//! Settles helper-discovered write capabilities in the authenticated hook. A checked result is consumed once.
//! Only no-effect receipts can be retried, bound by the helper to the same operation/document/revision.
use super::read_consent::Consent;
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_ask::chrome::Tier;

pub(crate) fn checks(args: &Value) -> bool {
    matches!(
        args["action"].as_str(),
        Some(
            "page.click"
                | "page.double_click"
                | "page.triple_click"
                | "page.middle_click"
                | "page.right_click"
                | "page.key"
                | "page.type"
                | "page.select"
                | "page.check"
                | "page.uncheck"
                | "page.drag"
        )
    )
}

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: Value,
) -> Result<tool_calls::receipts::Execution, String> {
    prepare_consented(shared, owner, source, call, args, None).await
}

pub(super) async fn prepare_consented(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: Value,
    consent: Option<Consent>,
) -> Result<tool_calls::receipts::Execution, String> {
    super::gateway::validate_execute_args(&args)?;
    let (args, reopened) =
        match super::reopen::resolve(shared, owner, "chrome_execute", args).await? {
            super::reopen::Resolved::Ready(args, reopened) => (args, reopened),
            super::reopen::Resolved::Refused(answer) => {
                return Ok(Box::pin(std::future::ready(Ok(answer))));
            }
        };
    let mut answer = run(shared, owner, source, call, args.clone(), consent).await?;
    super::receipt::stamp(shared, "chrome_execute", &args, &mut answer["vwMcpResult"]);
    super::output::guard(&mut answer["vwMcpResult"]);
    if let Some(reopened) = reopened {
        super::reopen::note(&mut answer, &reopened);
    }
    Ok(Box::pin(std::future::ready(Ok(answer))))
}

pub(super) async fn run(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: Value,
    consent: Option<Consent>,
) -> Result<Value, String> {
    run_clocked(shared, owner, source, call, args, consent, None).await
}

pub(super) async fn run_clocked(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: Value,
    mut consent: Option<Consent>,
    clock: Option<&super::batch_wait::Clock>,
) -> Result<Value, String> {
    let mut previous = None;
    let generation = crate::stops::generation(shared, owner);
    loop {
        let answer = super::gateway::call_one_preflighted(
            shared,
            owner,
            "chrome_execute",
            args.clone(),
            consent.as_ref(),
            previous.as_deref(),
        )
        .await?;
        let body = &answer["vwMcpResult"]["structuredContent"];
        let error = &body["untrustedPageContent"]["error"];
        if body["status"] != "failed" || error["code"] != "CAPABILITY_REQUIRED" {
            return Ok(answer);
        }
        let (tier, action) = match error["details"]["capability"].as_str() {
            Some("page.commit") => (Tier::AcceptEdits, Some("page.commit")),
            Some("page.edit") => (Tier::AcceptEdits, Some("page.type")),
            Some("auth.secrets") => (Tier::FullAccess, Some("page.type")),
            Some("raw.execute") => (Tier::FullAccess, Some("raw.execute")),
            _ => (
                super::mapping::required("chrome_execute", &args)
                    .unwrap_or(Tier::AcceptEdits)
                    .max(Tier::AcceptEdits),
                None,
            ),
        };
        if consent.is_some_and(|consent| consent.0 >= tier) {
            return Ok(answer);
        }
        let waiting = clock.map(super::batch_wait::Clock::pause);
        tool_calls::approve(shared,owner,source,call,json!({"kind":"approval","vwBrowserTier":tier as u8,"vwBrowserAction":action,
            "effect":{"kind":"tool","toolName":"mcp_vw.chrome_execute","input":args},"requiredPermissions":[]})).await?;
        drop(waiting);
        if crate::stops::generation(shared, owner) != generation {
            return Err(crate::stops::APPROVAL_ENDED.into());
        }
        consent = Some(Consent(tier));
        super::check::authorize_read(shared, owner, "chrome_execute", &args, consent.as_ref())?;
        super::gateway::validate_session(shared, owner, "chrome_execute", &args)?;
        // A script-driven click may already have run before its send was blocked. Never replay it.
        if error["details"]["noEffect"] != true {
            return Ok(answer);
        }
        let Some(operation) = body["operationId"].as_str().filter(|id| !id.is_empty()) else {
            return Ok(answer);
        };
        previous = Some(operation.to_owned());
    }
}
