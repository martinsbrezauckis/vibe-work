//! The egress decision for one request target (`host`, `port`): where the proxy may connect, or why not. Names are
//! resolved HERE, every answer is checked, and the proxy connects to exactly the addresses returned, so a DNS answer
//! cannot change between the check and the connection (DNS rebinding). A PUBLIC target sent on through the user's
//! own proxy (`upstream`) is always asked for the checked address too, so it cannot re-resolve the name into its
//! own private network. The original Host header and TLS server name are retained.
//!
//! Depends on: `ip`, `resolve`. Called by: `serve`.
//! Invariants:
//! - This computer is reachable only as exactly `127.0.0.1`, `[::1]` or `localhost`, and only when the request's
//!   [`Scope`] allows that port (Private Network Access idea: never Vibe Work's own ports, never
//!   from a page that came from the web, from a session's local page only its own and other sessions' attributed
//!   ports). Every other loopback spelling (`127.0.0.2`, `[::ffff:127.0.0.1]`, names that resolve there) is refused.
//! - A NAME whose answers include anything that is not public is refused (one bad answer refuses the whole name).
//! - `*.localhost` (the app's own origins on Windows: `tauri.localhost`, `ipc.localhost`,...) is refused.
//! - The one exception to "public only": a request from the local-network page the user opened reaches that page's
//!   own host when it is (or resolves to) a private-network or carrier-grade-NAT address
//!   ([`Scope::local_network`]); never loopback, never link-local.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::time::Duration;

use crate::ip::{IpClass, classify};
use crate::resolve::Resolve;

/// Longest DNS wait.
pub const RESOLVE_LIMIT: Duration = Duration::from_secs(5);

/// A boxed scope answer.
pub type ScopeFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Who is asking: decides whether a port on this computer may be reached by this request.
pub trait Scope: Send + Sync {
    /// `Ok` when the request may reach `127.0.0.1:<port>`; else the reason.
    fn loopback(&self, port: u16) -> ScopeFuture<'_>;

    /// True when the request comes from the local-network page the USER opened on exactly `host` (lowercase, no
    /// brackets): that host may answer with private-network addresses (never loopback or link-local). Default: no.
    fn local_network(&self, host: &str) -> bool {
        let _ = host;
        false
    }
}

/// A scope that never reaches this computer (the shared listener: requests of unknown webviews).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLoopback;

impl Scope for NoLoopback {
    fn loopback(&self, _port: u16) -> ScopeFuture<'_> {
        Box::pin(async {
            Err("servers on this computer are not reachable from this page".to_owned())
        })
    }
}

/// The host part of a request target, without IPv6 brackets.
#[must_use]
pub fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// Where to connect for `host:port`, or the refusal text.
///
/// # Errors
/// The reason, as text that names the host and the address class.
pub async fn decide(
    resolver: &dyn Resolve,
    scope: &dyn Scope,
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, String> {
    let host = bare_host(host);
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    let accepted_loopback = match lower.as_str() {
        "127.0.0.1" => Some(vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))]),
        "::1" => Some(vec![SocketAddr::from((Ipv6Addr::LOCALHOST, port))]),
        // `localhost.` (a trailing dot) is a different name for the policy: refused like the navigation hook does.
        "localhost" if !host.ends_with('.') => Some(vec![
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
        ]),
        _ => None,
    };
    if let Some(addrs) = accepted_loopback {
        scope.loopback(port).await?;
        return Ok(addrs);
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return match classify(ip) {
            IpClass::Public => Ok(vec![SocketAddr::new(ip, port)]),
            IpClass::Private | IpClass::SharedCgnat if scope.local_network(&lower) => {
                Ok(vec![SocketAddr::new(ip, port)])
            }
            IpClass::Loopback => Err(format!(
                "{ip} is not a spelling of this computer that is opened (only 127.0.0.1, [::1] and localhost)"
            )),
            class => Err(format!("{ip} is {}", class.describe())),
        };
    }
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(format!("{lower} is reserved for this computer"));
    }
    if lower.is_empty() || lower.len() > 253 {
        return Err("the host name is not valid".to_owned());
    }
    let answers = match tokio::time::timeout(RESOLVE_LIMIT, resolver.resolve(&lower, port)).await {
        Ok(Ok(answers)) => answers,
        Ok(Err(error)) => return Err(format!("{lower} could not be resolved ({error})")),
        Err(_) => return Err(format!("{lower} did not resolve within 5 s")),
    };
    if answers.is_empty() {
        return Err(format!("{lower} has no address"));
    }
    // The user's own local-network page may resolve to private / carrier-grade-NAT addresses; nothing else may.
    let own_lan = scope.local_network(&lower);
    let acceptable = |class: IpClass| {
        class.is_public() || (own_lan && matches!(class, IpClass::Private | IpClass::SharedCgnat))
    };
    if let Some(bad) = answers.iter().find(|a| !acceptable(classify(a.ip()))) {
        return Err(format!(
            "{lower} resolves to {}, {}",
            bad.ip(),
            classify(bad.ip()).describe()
        ));
    }
    Ok(answers
        .into_iter()
        .map(|a| SocketAddr::new(a.ip(), port))
        .collect())
}
