//! A fake Mistral API on loopback for the model proxy tests: records every request it gets (method,
//! path, query, `Authorization`, other header names, body length) and answers per path:
//! - `POST /v1/chat/completions`: an SSE stream of `chunks` events, `gap` apart (header
//!   `x-test-chunks` / `x-test-gap-ms`; default 3 chunks, no gap), or a big body of `x-test-bytes`;
//! - rate-limit headers (the fixture): a request header `x-test-ratelimit: <limit tokens>,<remaining tokens>,
//!   <limit requests>,<remaining requests>` (an empty piece is left out) makes the answer carry
//!   `x-ratelimit-limit-tokens-minute`, `-remaining-tokens-minute`, `-limit-req-minute`,
//!   `-remaining-req-minute`, plus `x-ratelimit-tokens-query-cost: 4242` and `x-test-other: other-value`
//!   (headers the proxy must not read); `x-test-ratelimit-raw: <name>=<value>;...` adds exact headers
//!   (duplicates, odd values);
//! - `GET /v1/models/redirect`: 302 to the `x-test-location` header's value;
//! - `GET /v1/models/err-<code>`: status `<code>` with a JSON body and `retry-after: 7`;
//! - any request whose `Authorization` contains `bad-key`: 401 (a revoked key);
//! - `POST /v1/ocr` (the fixture): records the JSON body (`ocr_bodies`), waits `ocr_delay_ms`, answers
//!   `ocr_status` when set (with a body that echoes a marker, to prove error texts never show bodies),
//!   else pages: 2 for a PDF `document_url`, 1 otherwise, each `fake page <n> of <mime>, <bytes> bytes`
//!   (bytes = decoded length), the model `mistral-ocr-fake` and `usage_info.pages_processed`;
//! - the speech paths (the fixture) and the realtime WebSocket: `fake_speech.rs`;
//! - anything else: 200 with `{"ok":true,"path":...}`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;

#[path = "fake_speech.rs"]
pub mod speech;
pub use speech::SpeechControl;

/// One request as the fake saw it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub authorization: Option<String>,
    pub header_names: Vec<String>,
    pub body_len: usize,
}

/// The fake's OCR switches and records (the fixture).
#[derive(Clone, Default)]
pub struct OcrControl {
    /// JSON bodies of every `/v1/ocr` request.
    pub bodies: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Delay before answering, in ms.
    pub delay_ms: Arc<std::sync::atomic::AtomicU64>,
    /// Status to answer with instead of pages (0: pages).
    pub status: Arc<std::sync::atomic::AtomicU16>,
}

/// The fake's shared state: requests seen, OCR and speech switches.
type FakeState = (Arc<Mutex<Vec<Seen>>>, OcrControl, SpeechControl);

/// The running fake.
pub struct FakeMistral {
    pub addr: SocketAddr,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub ocr: OcrControl,
    pub speech: SpeechControl,
}

impl FakeMistral {
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn header_num(req: &axum::http::HeaderMap, name: &str, default: u64) -> u64 {
    req.get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The `/v1/ocr` answer (see the module header).
async fn ocr_answer(ocr: &OcrControl, body: &[u8]) -> Response {
    use std::sync::atomic::Ordering;
    let json: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    ocr.bodies.lock().unwrap().push(json.clone());
    let delay = ocr.delay_ms.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let status = ocr.status.load(Ordering::SeqCst);
    if status != 0 {
        return Response::builder()
            .status(StatusCode::from_u16(status).unwrap())
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"ECHOED-BODY-MARKER"}"#))
            .unwrap();
    }
    let doc = &json["document"];
    let url = doc["document_url"]
        .as_str()
        .or_else(|| doc["image_url"].as_str())
        .unwrap_or_default();
    let (mime, data) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .unwrap_or(("?", ""));
    let bytes = data.len() / 4 * 3 - data.bytes().rev().take_while(|&b| b == b'=').count();
    let count = if mime == "application/pdf" { 2 } else { 1 };
    let pages: Vec<serde_json::Value> = (0..count)
        .rev()
        .map(|i| {
            serde_json::json!({"index": i, "markdown": format!("fake page {} of {mime}, {bytes} bytes", i + 1),
                "images": [], "dimensions": null})
        })
        .collect();
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"pages": pages, "model": "mistral-ocr-fake",
                "document_annotation": null,
                "usage_info": {"pages_processed": count, "doc_size_bytes": bytes}})
            .to_string(),
        ))
        .unwrap()
}

async fn handle(State((seen, ocr, speech)): State<FakeState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    seen.lock().unwrap().push(Seen {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(str::to_owned),
        authorization: parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        header_names: parts
            .headers
            .keys()
            .map(|k| k.as_str().to_owned())
            .collect(),
        body_len: bytes.len(),
    });
    let path = parts.uri.path().to_owned();
    let bad_key = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("bad-key"));
    if bad_key {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"message":"Unauthorized","request_id":"fake"}"#,
            ))
            .unwrap();
    }
    if let Some(response) = speech::redirect(&speech, &path) {
        return response;
    }
    if let Some(response) = speech::answer(
        &speech,
        parts.method.as_str(),
        &path,
        parts.uri.query(),
        &parts.headers,
        &bytes,
    ) {
        return response;
    }
    if path == "/v1/ocr" && parts.method == axum::http::Method::POST {
        return ocr_answer(&ocr, &bytes).await;
    }
    if path == "/v1/models/redirect" {
        let to = parts
            .headers
            .get("x-test-location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("/")
            .to_owned();
        return Response::builder()
            .status(StatusCode::FOUND)
            .header("location", to)
            .body(Body::from("moved"))
            .unwrap();
    }
    if let Some(code) = path.strip_prefix("/v1/models/err-") {
        let code: u16 = code.parse().unwrap();
        let mut response = Response::builder()
            .status(StatusCode::from_u16(code).unwrap())
            .header("content-type", "application/json")
            .header("retry-after", "7")
            .body(Body::from(format!(
                r#"{{"object":"error","message":"fake {code}"}}"#
            )))
            .unwrap();
        add_rate_limit_headers(&parts.headers, response.headers_mut());
        return response;
    }
    if path == "/v1/chat/completions" {
        let mut response = chat_answer(&parts.headers);
        add_rate_limit_headers(&parts.headers, response.headers_mut());
        return response;
    }
    Response::builder()
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"ok": true, "path": path, "data": [{"id": "m1"}, {"id": "m2"}]})
                .to_string(),
        ))
        .unwrap()
}

/// Adds the headers the `x-test-ratelimit*` request headers ask for (see the module header).
fn add_rate_limit_headers(request: &axum::http::HeaderMap, out: &mut axum::http::HeaderMap) {
    use axum::http::{HeaderName, HeaderValue};
    if let Some(spec) = request
        .get("x-test-ratelimit")
        .and_then(|v| v.to_str().ok())
    {
        let names = [
            "x-ratelimit-limit-tokens-minute",
            "x-ratelimit-remaining-tokens-minute",
            "x-ratelimit-limit-req-minute",
            "x-ratelimit-remaining-req-minute",
        ];
        for (name, piece) in names.iter().zip(spec.split(',')) {
            if !piece.is_empty() {
                out.append(
                    HeaderName::from_static(name),
                    HeaderValue::from_str(piece).unwrap(),
                );
            }
        }
        out.append(
            "x-ratelimit-tokens-query-cost",
            HeaderValue::from_static("4242"),
        );
        out.append("x-test-other", HeaderValue::from_static("other-value"));
    }
    if let Some(raw) = request
        .get("x-test-ratelimit-raw")
        .and_then(|v| v.to_str().ok())
    {
        for pair in raw.split(';').filter(|p| !p.is_empty()) {
            let (name, value) = pair.split_once('=').unwrap();
            out.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
    }
}

/// The chat completions answer: a big body or an SSE stream (see the module header).
fn chat_answer(headers: &axum::http::HeaderMap) -> Response {
    let big = header_num(headers, "x-test-bytes", 0);
    if big > 0 {
        let chunk = Bytes::from(vec![b'x'; 64 * 1024]);
        let count = big / (64 * 1024);
        let stream =
            futures_util::stream::iter((0..count).map(move |_| Ok::<_, Infallible>(chunk.clone())));
        return Response::builder()
            .header("content-type", "application/octet-stream")
            .body(Body::from_stream(stream))
            .unwrap();
    }
    let chunks = header_num(headers, "x-test-chunks", 3);
    let gap = Duration::from_millis(header_num(headers, "x-test-gap-ms", 0));
    let stream = futures_util::stream::unfold(0u64, move |n| async move {
        if n >= chunks {
            return None;
        }
        if n > 0 && !gap.is_zero() {
            tokio::time::sleep(gap).await;
        }
        let event = format!("data: {{\"n\":{n}}}\n\n");
        Some((Ok::<_, Infallible>(Bytes::from(event)), n + 1))
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Starts the fake on 127.0.0.1.
pub async fn start() -> FakeMistral {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ocr = OcrControl::default();
    let speech = SpeechControl::default();
    let realtime = axum::Router::new()
        .route(
            "/v1/audio/transcriptions/realtime",
            axum::routing::get(speech::realtime),
        )
        .with_state(speech.clone());
    let app = axum::Router::new()
        .fallback(handle)
        .with_state((Arc::clone(&seen), ocr.clone(), speech.clone()))
        .merge(realtime);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    FakeMistral {
        addr,
        seen,
        ocr,
        speech,
    }
}
