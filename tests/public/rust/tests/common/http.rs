//! A minimal HTTP/1.1 client over raw TCP for the Debug API tests: full control over every header
//! (missing Host, hostile Origin, duplicate Authorization, absolute-form targets), chunked
//! decoding, and an SSE reader. Every body received is kept in a transcript so a test can scan
//! all route output for secrets.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::gate::lock;

/// One answer.
#[derive(Debug, Clone)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpReply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    /// The `error.code` of an error body.
    pub fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

fn decode_chunked(mut body: &str) -> String {
    let mut out = String::new();
    loop {
        let Some((size, rest)) = body.split_once("\r\n") else {
            return out;
        };
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
            return out;
        };
        if n == 0 || rest.len() < n {
            return out;
        }
        out.push_str(&rest[..n]);
        body = rest[n..].strip_prefix("\r\n").unwrap_or(&rest[n..]);
    }
}

fn parse(raw: &str) -> HttpReply {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        decode_chunked(body)
    } else {
        body.to_owned()
    };
    HttpReply {
        status,
        headers,
        body,
    }
}

/// True once `buf` holds a complete response: head plus `Content-Length` bytes, or a chunked body
/// up to its last chunk. (Reading to EOF is not enough: a server that refused a large body may
/// keep the connection open while it drains.)
fn complete(buf: &[u8]) -> bool {
    let text = String::from_utf8_lossy(buf);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let lower = head.to_ascii_lowercase();
    if let Some(len) = lower
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        return body.len() >= len;
    }
    lower.contains("transfer-encoding: chunked") && body.ends_with("0\r\n\r\n")
}

/// Sends `request` (complete text) and reads one complete response (or until the server
/// closes).
pub async fn raw(port: u16, request: &str) -> HttpReply {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    // A refused large body may be answered before it is fully sent; a failed write is fine then.
    let _ = stream.write_all(request.as_bytes()).await;
    let mut out = Vec::new();
    let read = async {
        let mut chunk = [0_u8; 8192];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
            }
            if complete(&out) {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), read)
        .await
        .expect("no answer within 60 s");
    parse(&String::from_utf8_lossy(&out))
}

/// A request builder with sensible defaults (loopback Host with the right port, no Origin).
pub fn request(
    method: &str,
    path: &str,
    port: u16,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> String {
    let mut text =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if let Some(token) = token {
        text.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    for (k, v) in extra {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(body) = body {
        text.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    text.push_str("\r\n");
    if let Some(body) = body {
        text.push_str(body);
    }
    text
}

/// An authorized client that records every body it receives.
#[derive(Clone)]
pub struct Client {
    pub port: u16,
    pub token: String,
    transcript: Arc<Mutex<Vec<String>>>,
}

impl Client {
    pub fn new(port: u16, token: &str) -> Self {
        Self {
            port,
            token: token.to_owned(),
            transcript: Arc::default(),
        }
    }
    pub async fn get(&self, path: &str) -> HttpReply {
        let reply = raw(
            self.port,
            &request("GET", path, self.port, Some(&self.token), &[], None),
        )
        .await;
        lock(&self.transcript).push(reply.body.clone());
        reply
    }
    pub async fn post(&self, path: &str, body: &Value) -> HttpReply {
        let text = body.to_string();
        let reply = raw(
            self.port,
            &request("POST", path, self.port, Some(&self.token), &[], Some(&text)),
        )
        .await;
        lock(&self.transcript).push(reply.body.clone());
        reply
    }
    /// `POST /commands`, asserting 200, returning the result JSON.
    pub async fn command(&self, command: Value) -> Value {
        let reply = self.post("/commands", &command).await;
        assert_eq!(reply.status, 200, "{command} -> {}", reply.body);
        reply.json()
    }
    /// Every body received so far (including SSE data recorded with `record`).
    pub fn transcript(&self) -> Vec<String> {
        lock(&self.transcript).clone()
    }
    pub fn record(&self, text: String) {
        lock(&self.transcript).push(text);
    }
    /// Opens `GET /events` (optionally for one session).
    pub async fn events(&self, session: Option<&str>) -> Sse {
        let path = session.map_or_else(|| "/events".to_owned(), |s| format!("/events?session={s}"));
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        let text = request("GET", &path, self.port, Some(&self.token), &[], None);
        stream.write_all(text.as_bytes()).await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut status_line = String::new();
        reader.read_line(&mut status_line).await.unwrap();
        assert!(status_line.contains(" 200 "), "events: {status_line}");
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if line == "\r\n" {
                break;
            }
        }
        Sse {
            reader,
            pending: String::new(),
            client: self.clone(),
        }
    }
}

/// One SSE event.
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: String,
    pub id: String,
    pub data: Value,
}

/// An open event stream (chunked body).
pub struct Sse {
    reader: BufReader<TcpStream>,
    pending: String,
    client: Client,
}

impl Sse {
    async fn read_chunk(&mut self) -> Option<()> {
        let mut size = String::new();
        if self.reader.read_line(&mut size).await.ok()? == 0 {
            return None;
        }
        let n = usize::from_str_radix(size.trim(), 16).ok()?;
        if n == 0 {
            return None;
        }
        let mut buf = vec![0_u8; n + 2];
        self.reader.read_exact(&mut buf).await.ok()?;
        self.pending.push_str(&String::from_utf8_lossy(&buf[..n]));
        Some(())
    }

    /// The next event (keep-alive comments skipped), or `None` at end of stream / timeout.
    pub async fn next(&mut self, limit: Duration) -> Option<SseEvent> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            while let Some(end) = self.pending.find("\n\n") {
                let block: String = self.pending.drain(..end + 2).collect();
                let mut event = SseEvent {
                    event: String::new(),
                    id: String::new(),
                    data: Value::Null,
                };
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        event.event = v.trim().to_owned();
                    } else if let Some(v) = line.strip_prefix("id:") {
                        event.id = v.trim().to_owned();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim_start());
                    }
                }
                if data.is_empty() {
                    continue;
                }
                self.client.record(data.clone());
                event.data = serde_json::from_str(&data).unwrap();
                return Some(event);
            }
            let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
            tokio::time::timeout(left, self.read_chunk()).await.ok()??;
        }
    }

    /// Reads until `pred` matches (returning that event) or `limit` passes (panics).
    ///
    /// The panic says how long it waited and names the last events seen: `next` returns `None` both at a
    /// timeout and at the end of the stream, and one Windows run under full-suite load failed here with no
    /// way to tell which (the fixture).
    pub async fn until(&mut self, limit: Duration, pred: impl Fn(&SseEvent) -> bool) -> SseEvent {
        let started = tokio::time::Instant::now();
        let deadline = started + limit;
        let mut seen: Vec<String> = Vec::new();
        loop {
            let left = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or_default();
            let Some(event) = (if left.is_zero() {
                None
            } else {
                self.next(left).await
            }) else {
                panic!(
                    "no matching event after {:?} of {limit:?} ({}); last events: {:?}",
                    started.elapsed(),
                    if started.elapsed() >= limit {
                        "timeout"
                    } else {
                        "stream ended"
                    },
                    &seen[seen.len().saturating_sub(8)..]
                );
            };
            if pred(&event) {
                return event;
            }
            let data: String = event.data.to_string().chars().take(120).collect();
            seen.push(format!("{} {data}", event.event));
        }
    }
}
