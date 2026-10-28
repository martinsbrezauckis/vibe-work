//! The `Origin` rule for a WebSocket handshake to this computer: it must come from a page on this
//! computer (`http(s)://localhost`, `127.0.0.1` or `[::1]`, any port). The port rules (a session's own and other
//! sessions' attributed ports, never Vibe Work's own) are `decide`'s and stay as they are; this adds what the
//! request barrier gives HTTP background requests on Windows: a page from the web does not reach this computer's
//! ports, also over a WebSocket (which no request barrier sees), even through a listener whose pages may reach them
//! (a frame or a page of another origin inside a session's own-server preview).
//!
//! The same rule holds for the local-network page the user opened (a Lan mount): a WebSocket handshake to its host must
//! come from that page itself (`Origin` host = the target host), so a frame of another origin inside that page
//! cannot open a WebSocket to the user's router either.
//!
//! Handshakes arrive two ways: in absolute form (`GET ws://...`, libsoup) with their headers in the proxy request, or
//! inside a `CONNECT` tunnel (Chromium sends every `ws://` through a proxy that way), where the guard reads the first
//! request head in the tunnel. A tunnel that does not start with an HTTP request (TLS: `wss://`, `https://`) is passed
//! on unchanged: its handshake cannot be read. A request head in a tunnel longer than [`MAX_HEAD`] is refused, never
//! passed on unread (a page controls the length of its WebSocket URL).
//!
//! Depends on: tokio io, url, `head`. Called by: `serve`.
//! Invariants: a refused handshake never reaches the server (the tunnel's server connection gets no byte); a
//! handshake without exactly one `Origin`, or with `null`, is refused; so is a tunnel's first request head that does
//! not end within [`MAX_HEAD`] bytes.

use tokio::io::{AsyncRead, AsyncReadExt};
use url::Url;

use crate::head::{MAX_HEAD, is_websocket_upgrade};

/// Whose pages may open a WebSocket to the target.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Rule<'a> {
    /// This computer: pages on this computer.
    ThisComputer,
    /// A local-network host (lower case, no brackets): only a page on that host.
    Host(&'a str),
}

/// `Ok` unless the headers are a WebSocket handshake whose `Origin` the `rule` does not accept.
///
/// # Errors
/// The refusal text.
pub(crate) fn check(headers: &[(String, String)], rule: Rule<'_>) -> Result<(), String> {
    if !is_websocket_upgrade(headers) {
        return Ok(());
    }
    let mut origins = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("origin"))
        .map(|(_, v)| v.trim());
    let (Some(origin), None) = (origins.next(), origins.next()) else {
        return Err("a WebSocket here needs the page's origin".to_owned());
    };
    let host = Url::parse(origin)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .and_then(|url| {
            url.host_str()
                .map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase())
        });
    match (rule, host.as_deref()) {
        (Rule::ThisComputer, Some("localhost" | "127.0.0.1" | "::1")) => Ok(()),
        (Rule::ThisComputer, _) => {
            Err("a page that is not on this computer cannot open a WebSocket to it".to_owned())
        }
        (Rule::Host(target), Some(page)) if page == target.trim_end_matches('.') => Ok(()),
        (Rule::Host(_), _) => {
            Err("only the local-network page itself can open a WebSocket to its device".to_owned())
        }
    }
}

/// The start of a tunnel to this computer: the bytes to send on (what the client sent first, `rest` included), or
/// the refusal when it is a WebSocket handshake that [`check`] refuses under `rule`. Reads at most one request head.
///
/// # Errors
/// The refusal text (also when the client closes or stalls before a full head, or the head is longer than
/// [`MAX_HEAD`]).
pub(crate) async fn tunnel_start(
    client: &mut (impl AsyncRead + Unpin),
    rest: Vec<u8>,
    rule: Rule<'_>,
) -> Result<Vec<u8>, String> {
    let mut buf = rest;
    let mut chunk = [0u8; 4096];
    if buf.is_empty() {
        let n = client
            .read(&mut chunk)
            .await
            .map_err(|_| "the tunnel closed".to_owned())?;
        buf.extend_from_slice(&chunk[..n]);
    }
    // An HTTP request starts with a method token (ASCII capitals); TLS starts with 0x16.
    if !buf.first().is_some_and(u8::is_ascii_uppercase) {
        return Ok(buf);
    }
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > MAX_HEAD {
            // Never passed on unread: the page sets the length (its URL), so a long head must not skip the check.
            return Err("the request head is too long".to_owned());
        }
        let n = client
            .read(&mut chunk)
            .await
            .map_err(|_| "the tunnel closed".to_owned())?;
        if n == 0 {
            return Err("the tunnel closed before a full request".to_owned());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..end]);
    let headers: Vec<(String, String)> = text
        .split("\r\n")
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    check(&headers, rule).map(|()| buf)
}
