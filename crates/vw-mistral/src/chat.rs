//! One short chat completion (`POST /v1/chat/completions`) for the core's own small text jobs (the
//! commit message draft). Not streaming, no tools, a low token cap.
//!
//! Depends on: reqwest, `serde_json`, `auth`. Called by: vw-keys (`KeyService::chat`).
//! Invariants: the key goes only into the zeroized `Authorization` header; no redirect is followed (the client's
//! policy); an error text never contains the key, the request or the response body (a status and a fixed
//! sentence only).

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{Value, json};

/// Why a chat call gave no text.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    /// The key cannot be put into a header.
    #[error("the API key is not valid text")]
    MalformedKey,
    /// Mistral answered with an HTTP error.
    #[error("Mistral answered {0}")]
    Status(u16),
    /// The request did not complete, or the answer had no text.
    #[error("{0}")]
    Network(String),
}

/// One request.
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    /// The model alias.
    pub model: &'a str,
    /// The system message.
    pub system: &'a str,
    /// The user message.
    pub user: &'a str,
    /// The answer's token cap.
    pub max_tokens: u32,
    /// The time limit.
    pub timeout: Duration,
    /// `reasoning_effort`, or `None` for the server default. The levels a model takes differ by model and change
    /// on the live API (Large 4 refuses `"none"` and always thinks); a request without the field is taken by all.
    pub reasoning_effort: Option<&'a str>,
}

/// Sends `request` to `api_base` with `key` and returns the answer's text.
///
/// # Errors
/// [`ChatError`].
pub async fn chat(
    client: &reqwest::Client,
    api_base: &str,
    key: &str,
    request: ChatRequest<'_>,
) -> Result<String, ChatError> {
    let ChatRequest {
        model,
        system,
        user,
        max_tokens,
        timeout,
        reasoning_effort,
    } = request;
    let auth = crate::auth::bearer(key).ok_or(ChatError::MalformedKey)?;
    let mut body = json!({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "max_tokens": max_tokens,
        "temperature": 0.2,
    });
    if let Some(effort) = reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    let body = body.to_string();
    let response = client
        .post(format!(
            "{}/v1/chat/completions",
            api_base.trim_end_matches('/')
        ))
        .header(AUTHORIZATION, auth)
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| {
            ChatError::Network(
                if e.is_timeout() {
                    "the request timed out"
                } else {
                    "the request failed"
                }
                .to_owned(),
            )
        })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(ChatError::Status(status));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| ChatError::Network("the answer could not be read".to_owned()))?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .as_ref()
        .and_then(answer_text)
        .ok_or_else(|| ChatError::Network("the answer had no text".to_owned()))
}

/// The text of `choices[0].message.content` (a string, or an array of chunks whose `text` parts are joined;
/// `thinking` chunks are skipped). Public for tests.
#[must_use]
pub fn answer_text(v: &Value) -> Option<String> {
    let content = v.pointer("/choices/0/message/content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => return None,
    };
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}
