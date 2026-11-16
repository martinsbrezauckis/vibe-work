//! Consent for a browser tool's tiered write package. The authenticated hook keeps the prepared call in its receipt.
//! Uses the existing card/grant service; no mode change, helper retry or model instructions.
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;

/// Core-only preflight marker; execution receives it only after consent. Never deserialized from tool input.
/// The second field records that the person answered a card naming a form send (see [`names_send`]).
#[derive(Clone, Copy)]
pub(crate) struct Consent(pub(super) vw_ask::chrome::Tier, pub(super) bool);

impl Consent {
    /// Whether this consent covers sending a form for this exact call: a card named the send, or the call's own
    /// card wording does.
    pub(super) fn sends(self, args: &Value) -> bool {
        self.1 || names_send(args)
    }
}

/// A write card whose own words name a form send: "Submit a form", "Accept a dialog", "Type … and submit it".
/// Any other write card ("Drag an element", "Press Enter", "Type into a field") approves that input only; a send
/// it causes is blocked by the helper before dispatch and asks again with the send's own card.
pub(super) fn names_send(args: &Value) -> bool {
    super::mapping::capability("chrome_execute", args) == Some("page.commit")
        || args["action"] == "page.type" && args["args"]["submit"] == true
}

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
    let browser = super::agent_browser::selected(shared, owner, "chrome_execute", &args);
    super::gateway::validate_execute_args(&args, browser)?;
    // Check availability, chat ownership and the current mode before opening any card.
    let consent = Consent(
        super::mapping::ask_required("chrome_execute", &args)
            .ok_or_else(|| super::browser_text::Fixed::UnknownAction.text(browser))?,
        false,
    );
    super::check::authorize_read(shared, owner, "chrome_execute", &args, Some(&consent))?;
    super::gateway::validate_session(shared, owner, "chrome_execute", &args)?;
    let detail = json!({"kind":"approval","vwBrowserTier":consent.0 as u8,"effect":{"kind":"tool","toolName":"mcp_vw.chrome_execute","input":args},"requiredPermissions":[]});
    tool_calls::approve(shared, owner, source, call, detail).await?;
    if matches!(args["action"].as_str(), Some("auth.fill" | "auth.basic")) {
        return super::login::prepare(shared, owner, args, Some(consent)).await;
    }
    if super::interaction_consent::checks(&args) {
        return super::interaction_consent::prepare(
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
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?;
        execute(&shared, &owner, args, consent).await
    }))
}

pub(super) async fn execute(
    shared: &Arc<Shared>,
    session: &str,
    args: Value,
    consent: Consent,
) -> Result<Value, String> {
    let request = args.clone();
    let mut answer = super::gateway::call_one_inner(
        shared,
        session,
        "chrome_execute",
        args,
        Some(&consent),
        None,
    )
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
    if tool == "chrome_execute"
        && args["action"]
            .as_str()
            .is_some_and(super::page_plan::handles)
    {
        return super::page_plan::send(shared, session, &args, consent).await;
    }
    if tool == "chrome_status"
        && args["action"]
            .as_str()
            .is_some_and(super::page_plan::handles)
    {
        let mut args = args;
        args.as_object_mut().map(|a| a.remove("action"));
        return Ok(shared.chrome_control.owner.call(tool, args).await);
    }
    if consent.is_some() {
        super::escalation::send(shared, session, tool, args, consent).await
    } else {
        super::login::send(shared, session, tool, args).await
    }
}
