//! Built-in browser activations run in Ask until the driver identifies POST activation, then use the existing card.
//! The hook pins the page and checked result, so the transport consumes it without replaying input.
use crate::{ask::tool_calls, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_agent_tools::wire::tools as names;
use vw_ask::chrome::Tier;

pub(super) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    args: &Value,
) -> Result<tool_calls::receipts::Execution, String> {
    let hub = crate::previews::hub(shared).map_err(|error| error.message)?;
    let page = crate::browser_tabs::agent_page(&hub.service, owner, args["tab"].as_str())
        .map_err(|error| error.message)?;
    let mut pinned = args.clone();
    pinned["tab"] = json!(page);
    let generation = crate::stops::generation(shared, owner);
    let answer = super::browser_tools::call_with_sending(
        shared,
        owner,
        names::BROWSER_ACT,
        pinned.clone(),
        false,
    )
    .await;
    let answer = match answer {
        Err(reason) if reason == vw_browser::SEND_REQUIRED => {
            tool_calls::approve(shared,owner,source,call,json!({"kind":"approval","vwBrowserTier":Tier::AcceptEdits as u8,
                "vwBrowserAction":crate::chrome_control::builtin::SEND_ACTION,"effect":{"kind":"tool","toolName":"mcp_vw.browser_act","input":args},"requiredPermissions":[]})).await?;
            if crate::stops::generation(shared, owner) != generation
                || crate::chrome_control::check::mode(shared, owner)
                    .as_deref()
                    .and_then(Tier::from_mode)
                    .is_none()
            {
                return Err(crate::stops::APPROVAL_ENDED.into());
            }
            super::browser_tools::check_current(shared, owner, &page)?;
            super::browser_tools::call_with_sending(shared, owner, names::BROWSER_ACT, pinned, true)
                .await?
        }
        answer => answer?,
    };
    Ok(Box::pin(std::future::ready(Ok(answer))))
}
