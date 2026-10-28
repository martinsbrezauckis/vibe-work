//! Carries Vibe's source/call identity to the core using the hook credential.
//! The strict pre-tool hook replaces any model-supplied transport receipt with a fresh core receipt.
use std::path::Path;

use serde_json::{Value, json};

use crate::wire::{self, tools};

pub(crate) fn decide(payload: &str, endpoint_file: &Path) -> Option<String> {
    let result = (|| {
        let payload: Value = serde_json::from_str(payload).map_err(|e| e.to_string())?;
        if !payload["tool_name"]
            .as_str()
            .is_some_and(|n| n.starts_with("mcp_vw."))
        {
            return Ok(None);
        }
        let endpoint = crate::hook::read_endpoint(endpoint_file)?;
        let input = wire::call(
            &endpoint.endpoint,
            &endpoint.credential,
            tools::TOOL_RECEIPT,
            payload,
            crate::hook::HOLD_TIMEOUT,
        )?;
        Ok::<_, String>(Some(
            json!({"hook_specific_output":{"tool_input":input}}).to_string(),
        ))
    })();
    match result {
        Ok(output) => output,
        Err(reason) => Some(json!({"decision":"deny","reason":reason}).to_string()),
    }
}
