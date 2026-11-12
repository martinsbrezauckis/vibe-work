//! HTTP parsing, fallback forwarding and streamed replies for the scripted model fixture.
//! Uses the parent's selected reply/state; called by its Axum server, never by production code.
use super::{Inner, Reply, Usage, edit_calls, pick, seen};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Lives through response production and its body; cancellation and EOF both release the observation.
struct ActiveReply(Option<Arc<Mutex<Inner>>>);
impl ActiveReply {
    fn new(inner: &Arc<Mutex<Inner>>, main: bool) -> Self {
        if main {
            inner.lock().unwrap().active_replies += 1;
        }
        Self(main.then(|| inner.clone()))
    }
}
impl Drop for ActiveReply {
    fn drop(&mut self) {
        if let Some(inner) = &self.0 {
            inner.lock().unwrap().active_replies -= 1;
        }
    }
}

fn chunk(id: u64, delta: &Value, finish: Option<&str>, usage: Option<Usage>) -> String {
    let mut chunk = json!({"id": format!("scripted-{id}"), "object": "chat.completion.chunk", "created": 1,
        "model": "scripted", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
    if let Some(u) = usage {
        chunk["usage"] = json!({"prompt_tokens": u.prompt, "completion_tokens": u.completion,
                                "total_tokens": u.prompt + u.completion});
    }
    format!("data: {chunk}\n\n")
}

/// Passes a request on to the fallback base unchanged (method, path and query, headers, body).
async fn forward(base: &str, request: axum::extract::Request, body: axum::body::Bytes) -> Response {
    let url = format!(
        "{base}{}",
        request.uri().path_and_query().map_or("/", |p| p.as_str())
    );
    let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut outgoing = reqwest::Client::new().request(method, url).body(body);
    for (name, value) in request.headers() {
        if name != "host" && name != "content-length" {
            outgoing = outgoing.header(name.as_str(), value.as_bytes());
        }
    }
    match outgoing.send().await {
        Ok(answer) => {
            let mut builder = Response::builder().status(answer.status().as_u16());
            for (name, value) in answer.headers() {
                if name != "content-length" && name != "transfer-encoding" {
                    builder = builder.header(name.as_str(), value.as_bytes());
                }
            }
            builder
                .body(axum::body::Body::from_stream(answer.bytes_stream()))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            format!("scripted model fallback: {error}"),
        )
            .into_response(),
    }
}

pub(super) async fn handle(
    State(inner): State<Arc<Mutex<Inner>>>,
    request: axum::extract::Request,
) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/v1/model-catalog"
        && let Some(catalog) = inner.lock().unwrap().script.model_catalog.clone()
    {
        return Json(catalog).into_response();
    }
    let authorized = {
        let state = inner.lock().unwrap();
        let bearer = request
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));
        bearer.is_some_and(|key| {
            !key.is_empty()
                && state
                    .expected_key
                    .as_deref()
                    .is_none_or(|expected| expected == key)
        })
    };
    if path == "/v1/chat/completions" && !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message":"Missing or incorrect model API key"})),
        )
            .into_response();
    }
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 64 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    // The provider ids the pinned Vibe sends: the agent's model, and its own narration summary model
    // (`vibe/app_server/_narration.py` `_NARRATION_MODEL`, `mistral-vibe-cli-fast`).
    if path == "/v1/chat/completions"
        && !matches!(
            body["model"].as_str(),
            Some(
                "mistral-large-4"
                    | "mistral-large-4-0"
                    | "mistral-vibe-cli-latest"
                    | "mistral-vibe-cli-fast"
                    | "zai-glm-5-3"
                    | "zai-glm-5-3-flash"
            )
        )
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"message":"Unknown model id (send the provider id, not its alias)"})),
        )
            .into_response();
    }
    let mut s = seen(&path, &body);
    s.query = parts.uri.query().map(str::to_owned);
    use sha2::Digest as _;
    s.authorization_sha256 = format!(
        "{:x}",
        sha2::Sha256::digest(
            parts
                .headers
                .get("authorization")
                .map_or(&[][..], |h| h.as_bytes())
        )
    );
    let registry = {
        let mut state = inner.lock().unwrap();
        state.calls += 1;
        state.log.push(s.clone());
        if state.script.platform
            && let Some(response) =
                state
                    .platform
                    .response(&parts.method, &parts.uri, &parts.headers, &body)
        {
            return response;
        }
        state.script.registry.clone()
    };
    if let Some(registry) = registry
        && let Some(response) = registry.response(&parts.method, &parts.uri)
    {
        return response;
    }
    // Decided under the lock, acted on after it (a guard must not live across an await).
    let decided = {
        let mut inner = inner.lock().unwrap();
        if path == "/v1/chat/completions" {
            static NEXT_REPLY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let id = (u64::from(std::process::id()) << 32)
                | NEXT_REPLY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok((pick(&mut inner, &s), id))
        } else {
            Err(inner.fallback.clone())
        }
    };
    let (mut reply, id) = match decided {
        Ok(found) => found,
        Err(Some(base)) => {
            let request = axum::extract::Request::from_parts(parts, axum::body::Body::empty());
            return forward(&base, request, bytes).await;
        }
        Err(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "not scripted"})),
            )
                .into_response();
        }
    };
    let active = ActiveReply::new(&inner, !s.side());
    if let Some(path) = reply.wait_file.clone() {
        loop {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(released) = serde_json::from_slice::<Reply>(&bytes) {
                    reply = released;
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    if reply.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(reply.delay_ms)).await;
    }
    if let Some(status) = reply.status {
        let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let message = reply
            .error
            .clone()
            .unwrap_or_else(|| "scripted error".to_owned());
        return (
            status,
            Json(json!({"object": "error", "message": message, "type": "scripted", "code": status.as_u16()})),
        )
            .into_response();
    }
    let text = reply
        .text
        .clone()
        .map(|t| t.replace("{user}", s.last_user()));
    let mut calls = reply.tools.clone();
    if reply.edit_tokens {
        calls.extend(edit_calls(&s, id));
    }
    let tool_calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let arguments = if reply.malformed && i == 0 {
                "{\"path\": ".to_owned()
            } else {
                c.args.to_string()
            };
            json!({"index": i, "id": format!("call{id}x{i}"), "type": "function",
                   "function": {"name": c.name, "arguments": arguments}})
        })
        .collect();
    let usage = reply.usage.unwrap_or(Usage {
        prompt: 1,
        completion: 1,
    });
    let finish = if tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    if !s.stream {
        let mut message = json!({"role": "assistant", "content": text.clone().unwrap_or_default()});
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }
        return Json(json!({"id": format!("scripted-{id}"), "object": "chat.completion", "created": 1,
            "model": "scripted", "choices": [{"index": 0, "message": message, "finish_reason": finish}],
            "usage": {"prompt_tokens": usage.prompt, "completion_tokens": usage.completion,
                      "total_tokens": usage.prompt + usage.completion}}))
        .into_response();
    }
    // Streamed: the text in `chunks` pieces, then the tool calls and the finish with the usage.
    let pieces = reply.chunks.max(1);
    let text = text.unwrap_or_default();
    let chars: Vec<char> = text.chars().collect();
    let size = chars.len().div_ceil(pieces).max(1);
    let mut frames: Vec<String> = chars
        .chunks(size)
        .map(|part| {
            chunk(
                id,
                &json!({"role": "assistant", "content": part.iter().collect::<String>()}),
                None,
                None,
            )
        })
        .collect();
    let last = if tool_calls.is_empty() {
        json!({"role": "assistant", "content": ""})
    } else {
        json!({"role": "assistant", "content": "", "tool_calls": tool_calls})
    };
    if let Some(reasoning) = reply.reasoning {
        frames.insert(0,chunk(id,&json!({"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":reasoning}]}]}),None,None));
    }
    frames.push(chunk(id, &last, Some(finish), Some(usage)));
    frames.push("data: [DONE]\n\n".to_owned());
    let gap = Duration::from_millis(reply.chunk_ms);
    let stream = futures_util::stream::unfold(
        (frames.into_iter().enumerate(), active),
        move |(mut it, active)| async move {
            let (i, frame) = it.next()?;
            if i > 0 && !gap.is_zero() {
                tokio::time::sleep(gap).await;
            }
            Some((Ok::<_, std::convert::Infallible>(frame), (it, active)))
        },
    );
    (
        [("content-type", "text/event-stream")],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}
