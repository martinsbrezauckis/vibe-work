//! Serving one listener of the [`crate::NetGuard`]: the accept loop and one connection (request head, canary,
//! decision, connect, relay). A CONNECT tunnel is one checked target by nature; a plain request goes through
//! `forward` (one checked target per client connection).
//!
//! After the decision, a PUBLIC target goes through the user's proxy when one applies (`upstream`); this
//! computer and the user's local-network page are always connected directly. A WebSocket handshake to this computer
//! must carry an `Origin` on this computer, and one to the user's local-network page that page's own origin
//! (`ws_origin`), in absolute form and inside a tunnel.
//!
//! Depends on: tokio, `guard` (shared state, scope), `decide`, `head`, `forward`, `upstream`, `ws_origin`. Called by:
//! `guard`.
//! Invariants: refusals are answered before any upstream connection (a tunnel's in-tunnel WebSocket refusal: the
//! server connection is open but gets no byte); the semaphore caps connections over all listeners; the head must
//! arrive within [`HEAD_LIMIT`].

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

/// Sends the existing plain refusal or unreachable page without reflecting raw HTML.
pub(crate) async fn answer(stream: &mut TcpStream, status: &str, body: &str) {
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

/// Answers a canary. A CONNECT name canary (macOS `WebKit` tunnels plain http too) is a real tunnel: the page goes
/// to the request sent through it, else the engine's load fails and it loads the canary again later.
async fn answer_canary(stream: &mut TcpStream, head: &head::Head, bare: &str, text: &str) {
    if matches!(head.target, Target::Tunnel { .. }) && bare.ends_with(CANARY_SUFFIX) {
        if stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .is_err()
        {
            return;
        }
        if tokio::time::timeout(HEAD_LIMIT, tunnelled_head(stream, head.rest.clone()))
            .await
            .is_err()
        {
            return;
        }
    }
    answer(stream, "200 OK", text).await;
}

/// Reads (and drops) the head of the request a client sends through a canary tunnel: up to the blank line, at most
/// 16 KiB, `already` being what arrived with the CONNECT.
async fn tunnelled_head(stream: &mut TcpStream, mut already: Vec<u8>) {
    let mut chunk = [0u8; 2048];
    while !already.windows(4).any(|w| w == b"\r\n\r\n") && already.len() < 16 * 1024 {
        match tokio::io::AsyncReadExt::read(stream, &mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => already.extend_from_slice(&chunk[..n]),
        }
    }
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
        answer_canary(&mut stream, &head, &bare, text).await;
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
    let decided = Decided {
        inner: &inner,
        who: &who,
        host: &host,
        bare: &bare,
        port,
    };
    let addrs = match decide(inner.resolver.as_ref(), &scope, &host, port).await {
        Ok(addrs) => addrs,
        Err(reason) => {
            answer(&mut stream, "403 Forbidden", &decided.refuse(&reason)).await;
            return;
        }
    };
    relay(&decided, stream, &head, &addrs, body).await;
}

/// A request the policy has judged: who asked, for which host and port.
struct Decided<'a> {
    inner: &'a Inner,
    who: &'a Who,
    /// The host as the request wrote it (IPv6 in brackets).
    host: &'a str,
    /// Lower case, no brackets.
    bare: &'a str,
    port: u16,
}

impl Decided<'_> {
    /// Records a refusal and answers its page text.
    fn refuse(&self, reason: &str) -> String {
        // Under the mount it came from (a store's only member; several members: none, see `Inner::refusal_key`).
        self.inner.record(
            self.inner.refusal_key(self.who).as_deref(),
            self.bare,
            self.port,
            reason,
        );
        format!("Vibe Work's browser does not open this address: {reason}.")
    }
}

/// The connection for a request the policy allowed: to `addrs`, or for a `public` target through the user's proxy
/// when one applies (the route). Every proxy is asked for a checked address, never the name. The error is the page text
/// of a 502.
async fn open(
    decided: &Decided<'_>,
    tunnel: bool,
    public: bool,
    addrs: &[SocketAddr],
) -> Result<(TcpStream, Option<crate::upstream::Route>), String> {
    let (host, bare, port) = (decided.host, decided.bare, decided.port);
    let route = if public {
        decided.inner.upstream_route(tunnel, host, port)
    } else {
        None
    };
    let Some(proxy) = &route else {
        let stream = connect(addrs)
            .await
            .map_err(|e| unreachable(bare, port, &e))?;
        return Ok((stream, None));
    };
    let through = if tunnel {
        crate::upstream::tunnel(proxy, addrs).await
    } else {
        crate::upstream::connect(proxy).await
    };
    match through {
        Ok(stream) => Ok((stream, route)),
        Err(error) => Err(format!(
            "{bare}:{port} could not be reached through your proxy: {error}."
        )),
    }
}

/// Connects a request the policy allowed (to `addrs`, or through the user's proxy for a public target) and relays it.
async fn relay(
    decided: &Decided<'_>,
    mut stream: TcpStream,
    head: &head::Head,
    addrs: &[SocketAddr],
    body: u64,
) {
    let bare = decided.bare;
    let refuse = |reason: &str| decided.refuse(reason);
    let local = addrs.iter().all(|a| a.ip().is_loopback());
    let public = addrs
        .iter()
        .all(|a| crate::ip::classify(a.ip()).is_public());
    // Who may open a WebSocket here: pages on this computer, or (the user's local-network page) that host only.
    let rule = if local {
        Some(crate::ws_origin::Rule::ThisComputer)
    } else if public {
        None
    } else {
        Some(crate::ws_origin::Rule::Host(bare))
    };
    if let Some(rule) = rule
        && let Target::Forward { headers, .. } = &head.target
        && let Err(reason) = crate::ws_origin::check(headers, rule)
    {
        answer(&mut stream, "403 Forbidden", &refuse(&reason)).await;
        return;
    }
    let tunnel = matches!(head.target, Target::Tunnel { .. });
    let (mut upstream, route) = match open(decided, tunnel, public, addrs).await {
        Ok(open) => open,
        Err(text) => {
            answer(&mut stream, "502 Bad Gateway", &text).await;
            return;
        }
    };
    match &head.target {
        Target::Direct { .. } => {}
        Target::Tunnel { .. } => {
            // One target by nature: the checked addresses (or the proxy's tunnel to a checked address), connected above.
            if stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
            let first = if let Some(rule) = rule {
                match tokio::time::timeout(
                    HEAD_LIMIT,
                    crate::ws_origin::tunnel_start(&mut stream, head.rest.clone(), rule),
                )
                .await
                {
                    Ok(Ok(first)) => first,
                    Ok(Err(reason)) => {
                        answer(&mut stream, "403 Forbidden", &refuse(&reason)).await;
                        return;
                    }
                    Err(_) => return,
                }
            } else {
                head.rest.clone()
            };
            if upstream.write_all(&first).await.is_err() {
                return;
            }
            let _ = copy_bidirectional(&mut stream, &mut upstream).await;
        }
        Target::Forward { headers, .. } => {
            let upgrade = headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("upgrade"));
            let request = match forward_head(&head.target, route.as_ref(), addrs) {
                Ok(request) => request,
                Err(reason) => {
                    answer(&mut stream, "502 Bad Gateway", &reason).await;
                    return;
                }
            };
            crate::forward::exchange(
                stream,
                upstream,
                &request,
                &head.rest,
                body,
                upgrade,
                route.is_some(),
            )
            .await;
        }
    }
}

/// The HTTP head for a direct or proxied request. Host is retained; every proxy target is a checked address.
fn forward_head(
    target: &Target,
    route: Option<&crate::upstream::Route>,
    addrs: &[SocketAddr],
) -> Result<Vec<u8>, String> {
    let Target::Forward {
        url,
        method,
        version,
        headers,
        ..
    } = target
    else {
        return Err("the guard expected an HTTP request".to_owned());
    };
    let Some(route) = route else {
        return Ok(head::origin_head(method, url, version, headers));
    };
    let to = addrs
        .first()
        .ok_or_else(|| "the guard has no checked address".to_owned())?;
    let url = crate::upstream::to_address(url, *to)?;
    Ok(head::proxy_head(
        method,
        &url,
        version,
        headers,
        route.auth.as_deref(),
    ))
}
