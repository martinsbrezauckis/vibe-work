//! Serving one listener of the [`crate::NetGuard`]: the accept loop and one connection (request head, canary,
//! decision, connect, relay). A CONNECT tunnel is one checked target by nature; a plain request goes through
//! `forward` (one checked target per client connection).
//!
//! Depends on: tokio, `guard` (shared state, scope), `decide`, `head`, `forward`. Called by: `guard`.
//! Invariants: refusals are answered before any upstream connection; the semaphore caps connections over all
//! listeners; the head must arrive within [`HEAD_LIMIT`].

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::decide::decide;
use crate::guard::{CANARY_SUFFIX, Inner, LOOPBACK_CANARY_PATH, ListenerScope, Who};
use crate::head::{self, Target};

/// Longest wait for a request head.
const HEAD_LIMIT: Duration = Duration::from_secs(15);
/// Longest upstream connect per address.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

/// Accepts connections on `listener` with `who`'s rights until aborted.
pub(crate) fn accept_loop(inner: Arc<Inner>, listener: TcpListener, who: Who) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            };
            let (inner, who) = (inner.clone(), who.clone());
            tokio::spawn(async move { serve_one(inner, stream, who).await });
        }
    })
}

async fn answer(stream: &mut TcpStream, status: &str, body: &str) {
    // the page title is what the Browser tab shows as the page's name. A server that did not answer was
    // titled "Blocked by Vibe Work" too, which read as a refusal; it is "Not reachable" now.
    let title = if status.starts_with("502") {
        "Not reachable"
    } else {
        "Blocked by Vibe Work"
    };
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title>\
         <p style=\"font:14px system-ui;margin:2em\">{}</p>",
        escape(body)
    );
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        page.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(page.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// The text of the page for an address that did not answer (plain words and what to do).
fn unreachable(bare: &str, port: u16, error: &str) -> String {
    format!(
        "{bare}:{port} did not answer ({error}). If it is your app's server, start it, then reload."
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn connect(addrs: &[SocketAddr]) -> Result<TcpStream, String> {
    let mut last = "no address".to_owned();
    for addr in addrs {
        match tokio::time::timeout(CONNECT_LIMIT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            // The kind in plain words ("connection refused"), without the system's error number.
            Ok(Err(e)) => last = e.kind().to_string(),
            Err(_) => "timed out".clone_into(&mut last),
        }
    }
    Err(last)
}

/// A canary request: marks its token and returns the answer text, else `None`.
///
/// The public canary is any request for `<token>.vw-guard.invalid` (only this proxy can answer that name). The
/// loopback canary is a request for THIS listener's own port that arrives in proxy form: it proves
/// the engine sends loopback traffic through the guard. Sent directly (an engine that bypasses the proxy for loopback),
/// it arrives in origin form and fails to parse (400): nothing is marked.
fn canary(
    inner: &Inner,
    target: &Target,
    stream: &TcpStream,
    bare: &str,
    port: u16,
) -> Option<&'static str> {
    if let Some(token) = bare.strip_suffix(CANARY_SUFFIX) {
        inner.saw(token);
        return Some("Vibe Work network guard: active.");
    }
    if let (Target::Forward { url, .. }, Ok(local)) = (target, stream.local_addr())
        && bare == "127.0.0.1"
        && port == local.port()
        && let Some(token) = url.path().strip_prefix(LOOPBACK_CANARY_PATH)
    {
        inner.saw(&format!("loopback:{token}"));
        return Some("Vibe Work network guard: loopback active.");
    }
    // An engine that proxies everything through CONNECT (macOS `nw_proxy_config_create_http_connect`) sends no
    // path: a tunnel to this listener's own loopback port proves the same for this listener (one per mount).
    if let (Target::Tunnel { .. }, Ok(local)) = (target, stream.local_addr())
        && bare == "127.0.0.1"
        && port == local.port()
    {
        inner.saw(&format!("loopback-port:{port}"));
        return Some("Vibe Work network guard: loopback active.");
    }
    None
}

async fn serve_one(inner: Arc<Inner>, mut stream: TcpStream, who: Who) {
    let Ok(_slot) = inner.slots.clone().try_acquire_owned() else {
        answer(
            &mut stream,
            "503 Service Unavailable",
            "Too many connections.",
        )
        .await;
        return;
    };
    let head = match tokio::time::timeout(HEAD_LIMIT, head::read(&mut stream)).await {
        Ok(Ok(head)) => head,
        Ok(Err(reason)) => {
            answer(&mut stream, "400 Bad Request", &reason).await;
            return;
        }
        Err(_) => return,
    };
    if let Target::Direct { token } = &head.target {
        inner.saw(&format!("direct:{token}"));
        answer(&mut stream, "404 Not Found", "Not a proxy request.").await;
        return;
    }
    let (host, port) = match &head.target {
        Target::Tunnel { host, port } => (host.clone(), *port),
        Target::Direct { .. } => return,
        Target::Forward { url, raw_host, .. } => {
            (raw_host.clone(), url.port_or_known_default().unwrap_or(80))
        }
    };
    let bare = crate::decide::bare_host(&host).to_ascii_lowercase();
    if let Some(text) = canary(&inner, &head.target, &stream, &bare, port) {
        answer(&mut stream, "200 OK", text).await;
        return;
    }
    // A plain request's body must have a declared length (one checked target per connection, `forward`).
    let body = match &head.target {
        Target::Forward { headers, .. } => match crate::forward::body_length(headers) {
            Ok(n) => n,
            Err(reason) => {
                answer(&mut stream, "411 Length Required", reason).await;
                return;
            }
        },
        Target::Tunnel { .. } | Target::Direct { .. } => 0,
    };
    let scope = ListenerScope {
        inner: inner.clone(),
        who: who.clone(),
    };
    let addrs = match decide(inner.resolver.as_ref(), &scope, &host, port).await {
        Ok(addrs) => addrs,
        Err(reason) => {
            inner.record(who.key(), &bare, port, &reason);
            let text = format!("Vibe Work's browser does not open this address: {reason}.");
            answer(&mut stream, "403 Forbidden", &text).await;
            return;
        }
    };
    let mut upstream = match connect(&addrs).await {
        Ok(upstream) => upstream,
        Err(error) => {
            answer(
                &mut stream,
                "502 Bad Gateway",
                &unreachable(&bare, port, &error),
            )
            .await;
            return;
        }
    };
    match &head.target {
        Target::Direct { .. } => {}
        Target::Tunnel { .. } => {
            // One target by nature: the checked addresses, connected above.
            if stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
                || upstream.write_all(&head.rest).await.is_err()
            {
                return;
            }
            let _ = copy_bidirectional(&mut stream, &mut upstream).await;
        }
        Target::Forward {
            url,
            method,
            version,
            headers,
            ..
        } => {
            let upgrade = headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("upgrade"));
            let request = head::origin_head(method, url, version, headers);
            crate::forward::exchange(stream, upstream, &request, &head.rest, body, upgrade).await;
        }
    }
}
