//! One non-streaming `POST /v1/conversations` with Mistral's built-in tools, and the parts of its answer
//! the app reads.
//!
//! Source: the `mistralai` 2.6.0 SDK (`ConversationRequest`: `model`, `inputs`, `instructions`, `tools`, `store`) and
//! live probes on. Measured answer shape:
//! - `outputs[]` holds `tool.execution` entries (`name` = the built-in tool, `function` = what it ran, e.g.
//!   `generate_image` / `edit_image` / `library_search`, `arguments` = a JSON text, `info.result` = a JSON text or
//!   an error sentence) and `message.output` entries (`content` = a string, or chunks of which we read `text`);
//! - `usage` carries `prompt_tokens`, `completion_tokens`, `connector_tokens` and `connectors` (calls per tool).
//!
//! We send `store: false`: the conversation is not kept on Mistral's side (the image files and the libraries are,
//! by their own APIs).
//!
//! Depends on: `serde_json`, `super` (request helper). Called by: `images`, `libraries`.
//! Invariant: nothing here interprets text from the answer as an instruction; the callers treat it as data.

use std::time::Duration;

use serde_json::{Value, json};

use super::{ApiCall, WorkError, json_body, json_call};

/// One tool run inside the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    /// The built-in tool (`image_generation`, `document_library`).
    pub name: String,
    /// What it ran (`generate_image`, `edit_image`, `library_search`).
    pub function: String,
    /// Its arguments (parsed; `Null` when they were not JSON).
    pub arguments: Value,
    /// Its result text (`info.result`): JSON for a success, a sentence for an error.
    pub result: String,
}

/// What a conversation returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The model's text, all `message.output` entries joined.
    pub text: String,
    /// The tool runs, in order.
    pub runs: Vec<ToolRun>,
    /// Mistral's `usage` object as given (tokens and tool calls), for cost reporting.
    pub usage: Value,
    /// The model that answered.
    pub model: String,
}

/// The request body (no key in it). `inputs` is a string or a list of entries.
#[must_use]
pub fn request_body(model: &str, instructions: &str, inputs: &Value, tools: &Value) -> Value {
    json!({
        "model": model,
        "instructions": instructions,
        "inputs": inputs,
        "tools": tools,
        "store": false,
    })
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(chunks) => chunks
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Reads a conversation answer.
///
/// # Errors
/// [`WorkError::Shape`] when there is no `outputs` list.
pub fn parse(body: &Value) -> Result<Outcome, WorkError> {
    let outputs = body
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or(WorkError::Shape("no outputs"))?;
    let mut text = Vec::new();
    let mut runs = Vec::new();
    let mut model = String::new();
    for entry in outputs {
        if model.is_empty()
            && let Some(m) = entry.get("model").and_then(Value::as_str)
        {
            m.clone_into(&mut model);
        }
        match entry.get("type").and_then(Value::as_str) {
            Some("message.output") => {
                let t = text_of(entry.get("content").unwrap_or(&Value::Null));
                if !t.trim().is_empty() {
                    text.push(t);
                }
            }
            Some("tool.execution") => {
                let s = |k: &str| {
                    entry
                        .get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                runs.push(ToolRun {
                    name: s("name"),
                    function: s("function"),
                    arguments: serde_json::from_str(&s("arguments")).unwrap_or(Value::Null),
                    result: entry
                        .pointer("/info/result")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
            _ => {}
        }
    }
    Ok(Outcome {
        text: text.join("\n\n"),
        runs,
        usage: body.get("usage").cloned().unwrap_or(Value::Null),
        model,
    })
}

/// Starts (and finishes) one conversation.
///
/// # Errors
/// [`WorkError`] for a failed request or an unreadable answer.
pub async fn run(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
    body: &Value,
    timeout: Duration,
) -> Result<Outcome, WorkError> {
    let answer = json_call(
        client,
        api_base,
        ApiCall {
            method: reqwest::Method::POST,
            path: "/v1/conversations".to_owned(),
            body: Some(json_body(body)),
            timeout,
            key,
        },
    )
    .await?;
    parse(&answer)
}
