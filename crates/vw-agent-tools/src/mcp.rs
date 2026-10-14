//! The stdio MCP server Vibe starts per session (`agentConfig.mcpServers`, name `vw`): newline-delimited
//! JSON-RPC 2.0 on stdin/stdout (the MCP stdio transport).
//!
//! `initialize`, `ping` and `tools/list` are answered here, without the core (Vibe discovers tools eagerly,
//! including when the agent has automatic approval). `tools/call` is forwarded to the core with the session's
//! credential ([`crate::wire::call`]); the
//! core's JSON comes back as one text block plus `structuredContent` (a screenshot's image, under
//! [`crate::wire::IMAGE_KEY`], as an MCP image block first), a refusal as a result whose text starts with
//! "Not run:" and gives the reason; post-input failures preserve their sent-input wording. A refusal is a result,
//! not `isError`, whose text Vibe 2.26.1's unified harness drops. The loop ends on stdin EOF: when
//! `vibe-app-server` goes away its pipe closes, so this process never outlives its harness (it runs in its
//! own session, outside the server's process group, `_mcp_transport.py` / MCP SDK `start_new_session`).
//!
//! Depends on: `wire`, `serde_json`. Called by: `main.rs` (`vw-agent-tools mcp`), tests (in-memory streams).
//! Invariants:
//! - One request at a time (Vibe's pool sends them one by one, `_mcp_pool.py:79-97`).
//! - Never panics on input; malformed lines get a JSON-RPC parse error (or are ignored if they carry no id).

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::wire::{self, tools};

/// MCP protocol versions this server speaks; the client's choice is echoed when listed, else the newest.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Forwards one tool call to the core: `(tool, arguments) -> core JSON or a plain-text reason`.
pub trait Forward {
    /// Calls `tool` with `args` on behalf of this session.
    ///
    /// # Errors
    /// The core's refusal or a transport problem, as plain text.
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String>;
}

impl<F: FnMut(&str, Value) -> Result<Value, String>> Forward for F {
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        self(tool, args)
    }
}

/// The tool list (names, descriptions the model reads through `search_tool_functions`, input schemas).
#[must_use]
pub fn tool_list() -> Value {
    crate::specs::tool_list()
}

fn reply(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(Value::Object(mut map)) if map.contains_key(wire::IMAGE_KEY) => {
            // A screenshot: the image as an MCP image block, the rest of the answer as text.
            let image = map.remove(wire::IMAGE_KEY).unwrap_or(Value::Null);
            let rest = Value::Object(map);
            let text = serde_json::to_string(&rest).unwrap_or_else(|_| "{}".to_owned());
            json!({
                "content": [
                    {
                        "type": "image",
                        "data": image.get("data").cloned().unwrap_or(Value::Null),
                        "mimeType": image.get("mimeType").cloned().unwrap_or_else(|| json!("image/png")),
                    },
                    {"type": "text", "text": text},
                ],
                "structuredContent": rest,
                "isError": false,
            })
        }
        Ok(value) => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned());
            let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
            if value.is_object() {
                result["structuredContent"] = value;
            }
            result
        }
        // WORKAROUND(vibe 2.26.1): the unified harness hands an `isError` result to the model only as
        // "mcp_tool_error: The MCP tool returned an error" (its text is dropped), so every
        // refusal of ours ("port 28104 belongs to another session", "pid must be a process id from processes_list")
        // was invisible and the model retried. A refusal is therefore a SUCCESS result whose text starts
        // with "Not run:". Remove when Vibe passes an error result's text to the model (a Vibe issue).
        Err(reason) => {
            let text = if reason.starts_with("Not run")
                || reason.starts_with(wire::UNKNOWN_OUTCOME)
                || reason.starts_with("Stopped or failed after native mouse input was sent.")
                || reason.starts_with("The browser act ended but its receipt could not be saved:")
            {
                reason
            } else {
                format!("Not run: {reason}")
            };
            json!({"content": [{"type": "text", "text": text}], "isError": false})
        }
    }
}

/// Answers one parsed message; `None` for notifications.
fn handle(message: &Value, forward: &mut impl Forward) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|v| VERSIONS.contains(v))
                .unwrap_or(VERSIONS[0]);
            reply(
                &id,
                &json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "vibe-work", "version": crate::version()},
                }),
            )
        }
        "ping" => reply(&id, &json!({})),
        "tools/list" => reply(&id, &json!({"tools": tool_list()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if !tools::SESSION_TOOLS.contains(&name) {
                return Some(error(&id, -32602, "unknown tool"));
            }
            let args = params
                .get("arguments")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            reply(&id, &tool_result(forward.call(name, args)))
        }
        _ => error(&id, -32601, "method not found"),
    })
}

/// Runs the server until `input` ends. Returns when stdin closes (or stdout breaks).
///
/// # Errors
/// The I/O error that ended the loop early (a closed stdout); stdin EOF is a normal `Ok(())`.
pub fn serve(
    input: impl BufRead,
    mut output: impl Write,
    mut forward: impl Forward,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message, &mut forward),
            Err(_) => Some(error(&Value::Null, -32700, "parse error")),
        };
        if let Some(answer) = answer {
            let mut text = serde_json::to_vec(&answer).unwrap_or_default();
            text.push(b'\n');
            output.write_all(&text)?;
            output.flush()?;
        }
    }
    Ok(())
}
