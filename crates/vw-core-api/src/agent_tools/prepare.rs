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
    // Without the Chrome package the check below refuses every Chrome tool with "not in this version" first.
    if tool == "chrome_execute" && shared.chrome_control.config.available() {
        crate::chrome_control::gateway::validate_execute_args(args)?;
    }
    if tool == names::SECRET_RUN {
        return super::secret_run::prepare(shared, tools, owner, args)
            .await
            .map(Some);
    }
    if tool == names::DOCUMENT_READ && !super::document_consent::registered(shared, owner, args) {
        return super::document_consent::prepare(shared, owner, source, call, args)
            .await
            .map(Some);
    }
    if names::CHROME_TOOLS.contains(&tool) {
        let decision = crate::chrome_control::check::check(
            shared,
            &json!({"sessionId":source,"parentSessionId":owner,"toolName":format!("mcp_vw.{tool}"),"toolInput":args}),
        );
        if decision["decision"] != "allow" {
            return Err(decision["reason"]
                .as_str()
                .unwrap_or("Chrome was not allowed.")
                .into());
        }
    }
    if tool == "chrome_execute"
        && args["steps"].as_array().is_some_and(|steps| {
            steps.iter().any(|step| {
                ["auth.fill", "auth.basic"].contains(&step["action"].as_str().unwrap_or_default())
            })
        })
    {
        return crate::chrome_control::batch::prepare(shared, owner, args.clone())
            .await
            .map(Some);
    }
    if tool == "chrome_execute"
        && ["auth.fill", "auth.basic"].contains(&args["action"].as_str().unwrap_or_default())
    {
        return crate::chrome_control::login::prepare(shared, owner, args.clone())
            .await
            .map(Some);
    }
    if tool == names::BROWSER_ACT
        && let Some(execution) = super::browser_tools::prepare(shared, owner, args).await?
    {
        return Ok(Some(execution));
    }
    super::tools::guard_start(shared, tools, owner, tool, args).await?;
    Ok(None)
}
