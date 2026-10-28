//! Permission checks run in the pre-tool hook; execution starts only after its decision is consumed.
//! Uses existing tool policies and keeps prepared secret/browser actions inside the core.
use crate::{ask::tool_calls::receipts::Execution, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_agent_tools::wire::tools as names;

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    source: &str,
    call: &str,
    tool: &str,
    args: &mut Value,
) -> Result<Option<Execution>, String> {
    let tools = shared.agent_tools.as_ref().ok_or("Tools unavailable.")?;
    if tool == names::SECRET_RUN {
        return super::secret_run::prepare(shared, tools, owner, args)
            .await
            .map(Some);
    }
    if tool == names::DOCUMENT_READ && !super::document_consent::registered(shared, owner, args) {
        super::document_consent::attach(shared, owner, source, call, args).await?;
    }
    if tool == names::USE_YOUR_CHROME || names::CHROME_TOOLS.contains(&tool) {
        let decision = crate::chrome_control::check::check(shared, json!({"sessionId":source,"parentSessionId":owner,"toolName":format!("mcp_vw.{tool}"),"toolInput":args})).await?;
        if decision["decision"] != "allow" {
            return Err(decision["reason"]
                .as_str()
                .unwrap_or("Chrome was not allowed.")
                .into());
        }
    }
    if tool == names::BROWSER_ACT
        && let Some(execution) = super::browser_tools::prepare(shared, owner, args).await?
    {
        return Ok(Some(execution));
    }
    super::tools::guard_start(shared, tools, owner, tool, args).await?;
    Ok(None)
}
