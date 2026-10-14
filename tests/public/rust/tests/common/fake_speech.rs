//! The speech part of the fake Mistral (the fixture), mounted by `fake_mistral`:
//! - `POST /v1/audio/speech`: records the JSON body (and, with `VW_FAKE_TTS_LOG`, appends its text to that file);
//!   answers `tts_status` when set (with a body that echoes a marker), else `{"audio_data": base64(a WAV of 1.5 s of
//!   silence at 16 kHz)}`;
//! - `GET /v1/audio/voices?limit&offset&type`: 5 voices (3 presets with slugs, 2 custom without), at most
//!   3 per page whatever `limit` says (so the client must follow the pagination);
//! - `POST /v1/audio/transcriptions`: records content type and body; answers `batch_status` when set, else
//!   `{"model": "voxtral-mini-fake", "text": <batch_text>, "language": <the form's language or null>}`;
//! - `GET /v1/audio/transcriptions/realtime` (WebSocket): records the `Authorization` header, the query and
//!   every message; `ws_mode` 0: `session.created`, then one `transcription.text.delta` word of `ws_text`
//!   per `input_audio.append`, and on `input_audio.end` the remaining words then `transcription.done`;
//!   1: refuse the upgrade with 401; 2: send an `error` event after the first append; 3: close right after
//!   `session.created`;
//! - `redirect_all`: every request except `/v1/redirect-target` gets 307 to `/v1/redirect-target`.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use futures_util::SinkExt as _;

/// One realtime connection as the fake saw it.
#[derive(Debug, Clone, Default)]
pub struct WsSeen {
    pub authorization: Option<String>,
    pub query: Option<String>,
    pub messages: Vec<serde_json::Value>,
}

/// Batch requests seen: content type and body.
pub type BatchSeen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Switches and records of the speech fake.
#[derive(Clone)]
pub struct SpeechControl {
    pub tts_bodies: Arc<Mutex<Vec<serde_json::Value>>>,
    pub tts_status: Arc<AtomicU16>,
    pub batch: BatchSeen,
    pub batch_status: Arc<AtomicU16>,
    pub batch_text: Arc<Mutex<String>>,
    pub ws: Arc<Mutex<Vec<WsSeen>>>,
    pub ws_mode: Arc<AtomicU8>,
    pub ws_text: Arc<Mutex<String>>,
    pub redirect_all: Arc<AtomicBool>,
}

impl Default for SpeechControl {
    fn default() -> Self {
        Self {
            tts_bodies: Arc::default(),
            tts_status: Arc::default(),
            batch: Arc::default(),
            batch_status: Arc::default(),
            batch_text: Arc::new(Mutex::new("fake batch transcript".to_owned())),
            ws: Arc::default(),
            ws_mode: Arc::default(),
            ws_text: Arc::new(Mutex::new("hello from the fake realtime".to_owned())),
            redirect_all: Arc::default(),
        }
    }
}

/// A WAV of 1.5 s of silence at 16 kHz (what the fake TTS answers; long enough to see the narration bar).
pub fn tiny_wav() -> Vec<u8> {
    let samples = 24_000u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples * 2).to_le_bytes());
    out.extend(std::iter::repeat_n(0u8, (samples * 2) as usize));
    out
}

fn json(status: u16, body: serde_json::Value) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn marker(status: u16) -> Response {
    json(status, serde_json::json!({"message": "ECHOED-BODY-MARKER"}))
}

/// The 307 of `redirect_all`, when it is on and the path is not the target.
pub fn redirect(control: &SpeechControl, path: &str) -> Option<Response> {
    if !control.redirect_all.load(Ordering::SeqCst) || path == "/v1/redirect-target" {
        return None;
    }
    Some(
        Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header("location", "/v1/redirect-target")
            .body(Body::from("moved"))
            .unwrap(),
    )
}

/// Answers a speech REST path, or `None` when the path is not one of them.
pub fn answer(
    control: &SpeechControl,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Option<Response> {
    match (method, path) {
        ("POST", "/v1/audio/speech") => {
            let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
            // `VW_FAKE_TTS_LOG=<file>` (the fixture e2e): one JSON line per request with the text to speak, so the
            // browser test sees exactly what voice chat sent (the in-process record is not reachable from it).
            if let Some(path) = std::env::var_os("VW_FAKE_TTS_LOG") {
                use std::io::Write as _;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(
                        file,
                        "{}",
                        serde_json::json!({"input": value["input"], "voice": value["voice_id"]})
                    );
                }
            }
            control.tts_bodies.lock().unwrap().push(value);
            let status = control.tts_status.load(Ordering::SeqCst);
            if status != 0 {
                return Some(marker(status));
            }
            let audio = base64::engine::general_purpose::STANDARD.encode(tiny_wav());
            Some(json(200, serde_json::json!({"audio_data": audio})))
        }
        ("GET", "/v1/audio/voices") => {
            let params: std::collections::HashMap<String, String> =
                url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
                    .into_owned()
                    .collect();
            let offset: usize = params
                .get("offset")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let all = [
                ("Jane - Neutral", Some("gb_jane_neutral"), None::<&str>),
                ("Paul - Happy", Some("en_paul_happy"), None),
                ("Oliver - Calm", Some("gb_oliver_calm"), None),
                ("My voice", None, Some("user-1")),
                ("Another", None, Some("user-1")),
            ];
            let items: Vec<serde_json::Value> = all
                .iter()
                .enumerate()
                .skip(offset)
                .take(3)
                .map(|(i, (name, slug, user))| {
                    serde_json::json!({"id": format!("00000000-0000-0000-0000-00000000000{i}"),
                        "name": name, "slug": slug, "languages": ["en_gb"], "gender": null,
                        "user_id": user, "created_at": "2026-10-09T00:00:00Z"})
                })
                .collect();
            Some(json(
                200,
                serde_json::json!({"items": items, "total": all.len(), "page": offset / 3 + 1,
                    "page_size": 3, "total_pages": 2}),
            ))
        }
        ("POST", "/v1/audio/transcriptions") => {
            let content_type = headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            control
                .batch
                .lock()
                .unwrap()
                .push((content_type, body.to_vec()));
            let status = control.batch_status.load(Ordering::SeqCst);
            if status != 0 {
                return Some(marker(status));
            }
            let text = String::from_utf8_lossy(body);
            let language = text
                .split("name=\"language\"\r\n\r\n")
                .nth(1)
                .and_then(|rest| rest.split("\r\n").next())
                .map(str::to_owned);
            Some(json(
                200,
                serde_json::json!({"model": "voxtral-mini-fake",
                    "text": format!(" {} ", control.batch_text.lock().unwrap()),
                    "language": language}),
            ))
        }
        _ => None,
    }
}

/// The realtime WebSocket route.
pub async fn realtime(
    axum::extract::State(control): axum::extract::State<SpeechControl>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    upgrade: WebSocketUpgrade,
) -> Response {
    if control.ws_mode.load(Ordering::SeqCst) == 1 {
        return (StatusCode::UNAUTHORIZED, "no").into_response();
    }
    let seen = WsSeen {
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        query: uri.query().map(str::to_owned),
        messages: Vec::new(),
    };
    let index = {
        let mut all = control.ws.lock().unwrap();
        all.push(seen);
        all.len() - 1
    };
    upgrade.on_upgrade(move |socket| run(control, index, socket))
}

async fn send(socket: &mut WebSocket, value: serde_json::Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

async fn run(control: SpeechControl, index: usize, mut socket: WebSocket) {
    let mode = control.ws_mode.load(Ordering::SeqCst);
    let created = serde_json::json!({"type": "session.created", "session": {"request_id": "ws-fake",
        "model": "voxtral-mini-transcribe-realtime-2602",
        "audio_format": {"encoding": "pcm_s16le", "sample_rate": 16000}}});
    if !send(&mut socket, created).await {
        return;
    }
    if mode == 3 {
        let _ = socket.close().await;
        return;
    }
    let words: Vec<String> = control
        .ws_text
        .lock()
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut next = 0usize;
    while let Some(Ok(message)) = socket.recv().await {
        let Message::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap_or_default();
        control.ws.lock().unwrap()[index]
            .messages
            .push(value.clone());
        match value["type"].as_str().unwrap_or_default() {
            "session.update" => {
                let updated = serde_json::json!({"type": "session.updated",
                    "session": value["session"].clone()});
                if !send(&mut socket, updated).await {
                    return;
                }
            }
            "input_audio.append" => {
                if mode == 2 {
                    let error = serde_json::json!({"type": "error",
                        "error": {"message": "fake realtime failure", "code": 9}});
                    let _ = send(&mut socket, error).await;
                    let _ = socket.close().await;
                    return;
                }
                if next < words.len() {
                    let piece = if next == 0 {
                        words[0].clone()
                    } else {
                        format!(" {}", words[next])
                    };
                    next += 1;
                    if !send(
                        &mut socket,
                        serde_json::json!({"type": "transcription.text.delta", "text": piece}),
                    )
                    .await
                    {
                        return;
                    }
                }
            }
            "input_audio.end" => {
                while next < words.len() {
                    let piece = if next == 0 {
                        words[0].clone()
                    } else {
                        format!(" {}", words[next])
                    };
                    next += 1;
                    let _ = send(
                        &mut socket,
                        serde_json::json!({"type": "transcription.text.delta", "text": piece}),
                    )
                    .await;
                }
                let done = serde_json::json!({"type": "transcription.done",
                    "model": "voxtral-mini-transcribe-realtime-2602", "text": words.join(" "),
                    "language": null, "usage": {}});
                let _ = send(&mut socket, done).await;
                let _ = socket.close().await;
                return;
            }
            _ => {}
        }
    }
}
