//! One plain (`http://`, absolute-form) request through the guard: ONE checked target per client connection
//! (because a keep-alive client could otherwise send a second request, to another origin, down a
//! connection that was checked for the first one).
//!
//! Depends on: tokio io, `head`. Called by: `proxy`.
//! Invariants:
//! - Client → upstream carries only the request head and its declared body (`Content-Length`); a request with
//!   `Transfer-Encoding` is refused before any connection (411), bytes after the body are never forwarded.
//! - The response head the client gets says `Connection: close` and `Proxy-Connection: close`; the client socket is
//!   closed when the upstream answer ends or the client goes away. Later client bytes are read and dropped.
//! - An `Upgrade` request becomes a two-way tunnel only when the upstream answers `101`; any other answer is relayed
//!   like a plain one.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, copy};
use tokio::net::TcpStream;

use crate::head::MAX_HEAD;

/// The declared request body length.
///
/// # Errors
/// A reason when the body is chunked / transfer-encoded or the length is unreadable or ambiguous.
pub fn body_length(headers: &[(String, String)]) -> Result<u64, &'static str> {
    if headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("transfer-encoding"))
    {
        return Err("request bodies without a length are not forwarded");
    }
    let mut lengths = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.trim().parse::<u64>());
    let first = match lengths.next() {
        None => return Ok(0),
        Some(Ok(n)) => n,
        Some(Err(_)) => return Err("unreadable Content-Length"),
    };
    if lengths.any(|l| l != Ok(first)) {
        return Err("conflicting Content-Length headers");
    }
    Ok(first)
}

/// The status code of a response head (`HTTP/1.1 101 ...`).
fn status(head: &[u8]) -> Option<u16> {
    let line = head.split(|b| *b == b'\r').next()?;
    let text = std::str::from_utf8(line).ok()?;
    text.split(' ').nth(1)?.parse().ok()
}

/// `head` (without the blank line) with every connection header replaced by `Connection: close` and
/// `Proxy-Connection: close`.
#[must_use]
pub fn closing_head(head: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut out = String::with_capacity(text.len() + 48);
    for (i, line) in text.split("\r\n").enumerate() {
        let name = line
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if i > 0
            && matches!(
                name.as_str(),
                "connection" | "keep-alive" | "proxy-connection"
            )
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\nProxy-Connection: close\r\n\r\n");
    out.into_bytes()
}

/// Reads a response head; returns (head without the blank line, bytes after it).
async fn read_head(reader: &mut (impl AsyncRead + Unpin)) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(at + 4);
            buf.truncate(at);
            return Some((buf, rest));
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = reader.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Sends the request (head, `rest` up to `body` bytes, then the remaining body from the client) and relays the
/// answer; returns when the exchange is over (both sockets are dropped by the caller).
pub async fn exchange(
    mut client: TcpStream,
    mut upstream: TcpStream,
    request_head: &[u8],
    rest: &[u8],
    body: u64,
    upgrade: bool,
) {
    let early = usize::try_from(body).map_or(rest.len(), |b| b.min(rest.len()));
    if upstream.write_all(request_head).await.is_err()
        || upstream.write_all(&rest[..early]).await.is_err()
    {
        return;
    }
    let remaining = body.saturating_sub(early as u64);
    if remaining > 0
        && copy(&mut (&mut client).take(remaining), &mut upstream)
            .await
            .is_err()
    {
        return;
    }
    let Some((head, after)) = read_head(&mut upstream).await else {
        let _ = client
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return;
    };
    if upgrade && status(&head) == Some(101) {
        let mut whole = head;
        whole.extend_from_slice(b"\r\n\r\n");
        if client.write_all(&whole).await.is_err() || client.write_all(&after).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        return;
    }
    if client.write_all(&closing_head(&head)).await.is_err()
        || client.write_all(&after).await.is_err()
    {
        return;
    }
    let (mut client_read, mut client_write) = client.split();
    let relay = async {
        let _ = copy(&mut upstream, &mut client_write).await;
        let _ = client_write.shutdown().await;
    };
    tokio::select! {
        () = relay => {}
        () = drain(&mut client_read) => {}
    }
}

/// Reads and drops whatever the client still sends, until it closes.
async fn drain(reader: &mut (impl AsyncRead + Unpin)) {
    let mut sink = [0u8; 4096];
    while matches!(reader.read(&mut sink).await, Ok(n) if n > 0) {}
}
