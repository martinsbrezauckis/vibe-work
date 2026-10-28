//! Reading and rewriting one proxy request head (HTTP/1.x): `CONNECT host:port` for TLS (and secure websockets), or
//! an absolute-form request (`GET http://host:port/path HTTP/1.1`, or a plain WebSocket handshake written as
//! `GET ws://host:port/path` the way libsoup sends it) that is forwarded in origin form.
//!
//! Depends on: url, tokio (`AsyncRead`). Called by: `proxy`.
//! Invariants:
//! - A head is at most [`MAX_HEAD`] bytes and must arrive within the caller's timeout; bytes read past the head stay
//!   with the caller (`Head::rest`) and are sent on unchanged (a request body, a pipelined tunnel start).
//! - The forwarded head never carries `Proxy-*` or `Expect` headers; it asks the server to close after one answer unless the
//!   request is an `Upgrade` (websocket), so one client connection never reaches a second host.

use tokio::io::{AsyncRead, AsyncReadExt};
use url::Url;

/// Longest request head.
pub const MAX_HEAD: usize = 64 * 1024;

/// What the client asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `CONNECT host:port`.
    Tunnel {
        /// Host (IPv6 in brackets as written).
        host: String,
        /// Port.
        port: u16,
    },
    /// An origin-form request for the loopback canary path, sent to the guard's port DIRECTLY (not as a proxy):
    /// the engine bypassed the request barrier (the Windows check).
    Direct {
        /// The canary token.
        token: String,
    },
    /// An absolute-form `http://` request.
    Forward {
        /// The host exactly as the request target wrote it (before URL normalisation turns `127.1` into
        /// `127.0.0.1`): the guard decides on this spelling.
        raw_host: String,
        /// The full URL, always `http://` (a `ws://` handshake target is rewritten to its `http://` form).
        url: Url,
        /// The method.
        method: String,
        /// The HTTP version token (`HTTP/1.1`).
        version: String,
        /// Header lines (name, value) as received.
        headers: Vec<(String, String)>,
    },
}

/// A parsed head and the bytes after it.
#[derive(Debug)]
pub struct Head {
    /// The target.
    pub target: Target,
    /// Bytes already read past the head.
    pub rest: Vec<u8>,
}

/// Reads one head from `reader`.
///
/// # Errors
/// A short reason (closed early, too long, malformed).
pub async fn read(reader: &mut (impl AsyncRead + Unpin)) -> Result<Head, String> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(at) = find_end(&buf) {
            break at;
        }
        if buf.len() > MAX_HEAD {
            return Err("request head too long".to_owned());
        }
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(|e| format!("read failed: {e}"))?;
        if n == 0 {
            return Err("closed before a full request".to_owned());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let rest = buf.split_off(end + 4);
    buf.truncate(end);
    let text = String::from_utf8(buf).map_err(|_| "request head is not UTF-8".to_owned())?;
    parse(&text).map(|target| Head { target, rest })
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parses a head (without the final blank line).
///
/// # Errors
/// A short reason.
pub fn parse(text: &str) -> Result<Target, String> {
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed request line".to_owned());
    };
    if !version.starts_with("HTTP/1.") {
        return Err("only HTTP/1.x is spoken here".to_owned());
    }
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| "CONNECT needs host:port".to_owned())?;
        let port: u16 = port.parse().map_err(|_| "bad CONNECT port".to_owned())?;
        if host.is_empty() {
            return Err("CONNECT needs a host".to_owned());
        }
        return Ok(Target::Tunnel {
            host: host.to_owned(),
            port,
        });
    }
    if let Some(token) = target.strip_prefix(crate::guard::LOOPBACK_CANARY_PATH) {
        return Ok(Target::Direct {
            token: token.chars().take(64).collect(),
        });
    }
    let mut url =
        Url::parse(target).map_err(|_| "a proxy request needs an absolute URL".to_owned())?;
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "malformed header line".to_owned())?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    match url.scheme() {
        "http" => {}
        // A plain-text WebSocket handshake through an HTTP proxy: libsoup (WebKitGTK's network stack) writes the
        // target as `GET ws://host:port/path` (refusing it broke cross-session
        // WebSocket). It is the same request to the same host and port as its `http://` form (ws defaults to 80
        // like http), so it is judged and forwarded as that; only a real handshake is accepted in this form.
        "ws" if is_websocket_upgrade(&headers) => {
            url.set_scheme("http")
                .map_err(|()| "the WebSocket address could not be read".to_owned())?;
        }
        "ws" => return Err("a ws:// request must be a WebSocket handshake".to_owned()),
        _ => return Err("only http:// is forwarded (https goes through CONNECT)".to_owned()),
    }
    Ok(Target::Forward {
        raw_host: raw_host(target),
        url,
        method: method.to_owned(),
        version: version.to_owned(),
        headers,
    })
}

/// True when the headers ask for a WebSocket upgrade (`Upgrade: websocket`, any case).
pub(crate) fn is_websocket_upgrade(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(n, v)| {
        n.eq_ignore_ascii_case("upgrade")
            && v.split(',')
                .any(|p| p.trim().eq_ignore_ascii_case("websocket"))
    })
}

/// The host part of an absolute-form target as written (`http://[::1]:80/x` -> `[::1]`, user info dropped).
fn raw_host(target: &str) -> String {
    let after = target.split_once("://").map_or(target, |(_, rest)| rest);
    let authority = after.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if authority.starts_with('[') {
        return authority
            .split_once(']')
            .map_or(authority.to_owned(), |(h, _)| format!("{h}]"));
    }
    authority.split(':').next().unwrap_or("").to_owned()
}

/// The head sent to the user's proxy: absolute-form request line, the client's `Proxy-*` and hop-by-hop
/// headers dropped, ours (`Proxy-Authorization` when `auth`) added, `Connection: close` unless the request upgrades.
#[must_use]
pub fn proxy_head(
    method: &str,
    url: &Url,
    version: &str,
    headers: &[(String, String)],
    auth: Option<&str>,
) -> Vec<u8> {
    let origin = origin_head(method, url, version, headers);
    let text = String::from_utf8_lossy(&origin);
    let rest = text.split_once("\r\n").map_or("", |(_, rest)| rest);
    let mut out = format!("{method} {} {version}\r\n", url.as_str());
    if let Some(auth) = auth {
        out.push_str("Proxy-Authorization: ");
        out.push_str(auth);
        out.push_str("\r\n");
    }
    out.push_str(rest);
    out.into_bytes()
}

/// The head sent to the origin server: origin-form request line, `Proxy-*` / hop-by-hop headers dropped,
/// `Connection: close` unless the request upgrades.
#[must_use]
pub fn origin_head(
    method: &str,
    url: &Url,
    version: &str,
    headers: &[(String, String)],
) -> Vec<u8> {
    let upgrade = headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("upgrade"));
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let mut out = format!("{method} {path} {version}\r\n");
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("proxy-")
            || matches!(lower.as_str(), "connection" | "keep-alive" | "expect")
        {
            continue;
        }
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.push_str(if upgrade {
        "Connection: Upgrade\r\n"
    } else {
        "Connection: close\r\n"
    });
    out.push_str("\r\n");
    out.into_bytes()
}
