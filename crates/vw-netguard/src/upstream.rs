//! The user's HTTP(S) proxy for public egress: after the guard's policy check, a request to a
//! PUBLIC host goes through this proxy when one applies to it, instead of connecting directly, so public pages load
//! on networks that only allow the proxy out.
//!
//! Where the proxy comes from: Settings > Network (the core passes its values, [`Upstream::from_values`]), else the
//! system's ([`Upstream::system`]: the `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` environment, and on Windows
//! and macOS the system proxy settings, read by `hyper-util`'s matcher, the same rules reqwest uses for the core's own
//! clients). Only `http://` proxies are chained (plain HTTP absolute form, `CONNECT` for tunnels); a SOCKS or
//! `https://` proxy address is not used and such requests connect directly, as before.
//!
//! Depends on: hyper-util (`client::proxy::matcher`), http (`Uri`). Called by: `guard` (held), `serve` (per request).
//! Invariants:
//! - Asked only for public targets: this computer and the user's own local-network page are always reached
//!   directly, never through the proxy.
//! - Every proxy is asked for the ADDRESS the guard checked (`CONNECT 93.184.215.14:443`; plain HTTP in absolute
//!   form to that address with the page's `Host` header kept). Tunnel bytes, including TLS SNI, pass unchanged.
//!   The proxy never gets a hostname target to resolve again. If it refuses address targets, the request fails;
//!   there is no hostname fallback. Name-based proxy rules apply to the address instead (TLS SNI still has the name).
//! - The proxy's sign-in (user and password in its address) travels only as `Proxy-Authorization: Basic` to the
//!   proxy; it is never logged (this module logs nothing).

use std::net::SocketAddr;

use hyper_util::client::proxy::matcher::Matcher;

/// Where public requests go: the user's proxy rules (cheap to share behind an `Arc`).
pub struct Upstream {
    matcher: Matcher,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The addresses may carry a sign-in: never printed.
        f.debug_struct("Upstream").finish_non_exhaustive()
    }
}

/// One chained hop: the proxy's address and its sign-in header value.
#[derive(Clone)]
pub(crate) struct Route {
    /// Proxy host (no brackets).
    pub(crate) host: String,
    /// Proxy port.
    pub(crate) port: u16,
    /// `Basic ...` for `Proxy-Authorization`, when the address had a user.
    pub(crate) auth: Option<String>,
}

impl Upstream {
    /// The Settings > Network values (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; `None` = unset).
    #[must_use]
    pub fn from_values(
        http: Option<&str>,
        https: Option<&str>,
        all: Option<&str>,
        no: Option<&str>,
    ) -> Self {
        let value = |v: Option<&str>| v.unwrap_or_default().trim().to_owned();
        Self {
            matcher: Matcher::builder()
                .http(value(http))
                .https(value(https))
                .all(value(all))
                .no(value(no))
                .build(),
        }
    }

    /// The system's proxy: the process environment, then (Windows, macOS) the system proxy settings.
    #[must_use]
    pub fn system() -> Self {
        Self {
            matcher: Matcher::from_system(),
        }
    }

    /// No proxy at all.
    #[must_use]
    pub fn none() -> Self {
        Self::from_values(None, None, None, None)
    }

    /// The proxy hop for a public `host:port` reached over a tunnel (`tls`: `CONNECT`, the `HTTPS_PROXY` rule) or as
    /// plain HTTP (the `HTTP_PROXY` rule); `None`: connect directly (no rule, `NO_PROXY`, or a proxy kind other than
    /// `http://`).
    pub(crate) fn route(&self, tls: bool, host: &str, port: u16) -> Option<Route> {
        let scheme = if tls { "https" } else { "http" };
        let uri: http::Uri = format!("{scheme}://{host}:{port}/").parse().ok()?;
        let intercept = self.matcher.intercept(&uri)?;
        let proxy = intercept.uri();
        if proxy.scheme_str() != Some("http") {
            return None;
        }
        let proxy_host = proxy
            .host()?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        Some(Route {
            host: proxy_host,
            port: proxy.port_u16().unwrap_or(80),
            auth: intercept
                .basic_auth()
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        })
    }
}

/// Longest wait for the proxy's answer to `CONNECT`.
const ANSWER_LIMIT: std::time::Duration = std::time::Duration::from_secs(15);
/// Longest connect to the proxy.
const CONNECT_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// True when the connection from `local` to `peer` (a connected proxy) stays on this computer: the proxy's address
/// is loopback or unspecified, or it is the connection's own source address (a computer that connects to one of its
/// own addresses uses that address as the source: RFC 6724 rule 1, "prefer same address"). Opens no socket, so it
/// can never raise a firewall prompt. This diagnostic does not change address pinning: every proxy gets checked IPs.
#[must_use]
pub fn on_this_computer(local: SocketAddr, peer: SocketAddr) -> bool {
    let ip = peer.ip();
    ip.is_loopback() || ip.is_unspecified() || ip == local.ip()
}

/// A connection to the proxy of `route` (its name resolved by the system, every address tried in turn).
///
/// # Errors
/// A plain reason (no address, refused, timed out); the proxy's address is not in it.
pub(crate) async fn connect(route: &Route) -> Result<tokio::net::TcpStream, String> {
    let addrs = tokio::time::timeout(
        CONNECT_LIMIT,
        tokio::net::lookup_host((route.host.as_str(), route.port)),
    )
    .await
    .map_err(|_| "the proxy's name did not resolve in time".to_owned())?
    .map_err(|_| "the proxy's name could not be resolved".to_owned())?;
    let mut last = "the proxy has no address".to_owned();
    for addr in addrs {
        match tokio::time::timeout(CONNECT_LIMIT, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                return Ok(stream);
            }
            Ok(Err(e)) => last = format!("the proxy did not answer ({})", e.kind()),
            Err(_) => "the proxy did not answer in time".clone_into(&mut last),
        }
    }
    Err(last)
}

/// The checked HTTP request target; the request's `Host` header stays the page's.
/// An invalid target is an error, never a reason to send the original hostname.
pub(crate) fn to_address(url: &url::Url, to: SocketAddr) -> Result<url::Url, String> {
    let mut out = url.clone();
    out.set_ip_host(to.ip())
        .map_err(|()| "the checked proxy address could not be set".to_owned())?;
    out.set_port(Some(to.port()))
        .map_err(|()| "the checked proxy port could not be set".to_owned())?;
    Ok(out)
}

/// A tunnel through the proxy of `route`: `CONNECT <checked address>` (IPv6 in brackets), each checked address
/// in turn until one is accepted. TLS SNI and all other tunnel bytes pass unchanged. Never retries by hostname.
///
/// # Errors
/// A plain reason (the proxy could not be reached or refused the tunnel: its status code).
pub(crate) async fn tunnel(
    route: &Route,
    checked: &[SocketAddr],
) -> Result<tokio::net::TcpStream, String> {
    let mut last = "the guard has no checked address".to_owned();
    for addr in checked {
        let mut stream = connect(route).await?;
        match ask_tunnel(&mut stream, route, &addr.to_string()).await {
            Ok(()) => return Ok(stream),
            Err(reason) => last = reason,
        }
    }
    Err(last)
}

/// Sends `CONNECT target` on `stream` and reads the proxy's answer head (nothing after it).
async fn ask_tunnel(
    stream: &mut tokio::net::TcpStream,
    route: &Route,
    target: &str,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = &route.auth {
        request.push_str("Proxy-Authorization: ");
        request.push_str(auth);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| "the proxy closed the connection".to_owned())?;
    // Read the answer head byte by byte: nothing after it may be consumed (it belongs to the tunnel).
    let read = async {
        let mut head = Vec::with_capacity(256);
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 8192 || stream.read(&mut byte).await.unwrap_or(0) == 0 {
                return None;
            }
            head.push(byte[0]);
        }
        Some(head)
    };
    let head = tokio::time::timeout(ANSWER_LIMIT, read)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| "the proxy did not answer the tunnel request".to_owned())?;
    let status = String::from_utf8_lossy(&head)
        .split(' ')
        .nth(1)
        .unwrap_or("")
        .to_owned();
    if status == "200" {
        Ok(())
    } else {
        Err(format!(
            "the proxy refused the connection (status {status})"
        ))
    }
}
