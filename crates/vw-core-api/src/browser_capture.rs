//! Capture in the core: the shared path of the `browser_capture` agent tool and the `browser` command's
//! `capture` op (the window's Capture menu and the Debug API), and the media fetcher.
//!
//! The media fetcher allows only HTTP(S), checks every redirect hop (at most [`MAX_HOPS`]), and pins DNS
//! answers to the connection. A page whose current host is private may load media from other private hosts;
//! public pages retain the guard's public-only address policy. Loopback follows the session-port policy; link-local and reserved addresses
//! stay refused. Public media uses the browser guard's selected proxy and checked-IP route; permitted local media
//! stays direct. No cookies or page credentials; size and time are bounded.
//!
//! Depends on: vw-browser (`capture`), vw-preview (policy), vw-netguard (address classes), vw-mistral (TLS setup),
//! reqwest, `agent_tools::session` (the session folder), `previews::hub`. Called by: `browser` (command),
//! `agent_tools::browser_tools`.
//! Invariant: the capture folder is the session's first folder (`agent_tools::session::folders`); nothing is written
//! elsewhere (vw-browser `capture::files`).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use url::Url;
use vw_browser::capture::{self, Fetch, FetchFuture, Fetched, Target};
use vw_browser::{ActError, Actor};
use vw_netguard::ip::{IpClass, classify};
use vw_preview::{Destination, PreviewService};

use crate::core::Shared;
use crate::previews::hub;

/// Most redirect hops a media fetch follows (each one checked again).
pub(crate) const MAX_HOPS: usize = 3;

/// Resolves a host name and answers its addresses only when every one is public (the same rule as the
/// network guard's proxy).
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::lookup_host((host.as_str(), 0)),
            )
            .await
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "the name lookup timed out".into()
            })??
            .collect();
            if found.is_empty() {
                return Err("the name has no address".into());
            }
            if let Some(bad) = found.iter().find(|a| classify(a.ip()) != IpClass::Public) {
                return Err(format!(
                    "refused: {host} resolves to a non-public address ({})",
                    bad.ip()
                )
                .into());
            }
            Ok(Box::new(found.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

// HTTP libraries wrap DNS errors; inspect the source chain without exposing its addresses or values.
fn refused_lookup(mut error: &(dyn std::error::Error + 'static)) -> bool {
    loop {
        if error.to_string().contains("refused:") {
            return true;
        }
        let Some(source) = error.source() else {
            return false;
        };
        error = source;
    }
}

/// A refusal from this guard listener for this hop, without exposing proxy endpoints or credentials.
/// The second answer marks transport failures that must refuse the whole capture.
fn guard_refusal(guard: &vw_netguard::NetGuard, since: u64, url: &Url) -> Option<(String, bool)> {
    let host = url
        .host_str()?
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    guard
        .refusals_since(since)
        .into_iter()
        .rev()
        .find(|refusal| refusal.key.is_none() && refusal.host == host && refusal.port == port)
        .map(|refusal| {
            #[cfg(feature = "proxy-transports")]
            for error in [
                vw_netguard::upstream::Error::Configuration,
                vw_netguard::upstream::Error::Discovery,
                vw_netguard::upstream::Error::Unreachable,
                vw_netguard::upstream::Error::ProxyUnreachable,
                vw_netguard::upstream::Error::Timeout,
                vw_netguard::upstream::Error::Certificate,
                vw_netguard::upstream::Error::Authentication,
                vw_netguard::upstream::Error::Refused,
                vw_netguard::upstream::Error::Protocol,
            ] {
                if refusal.reason == error.outcome() {
                    return (format!("refused: {}", error.state()), true);
                }
            }
            #[cfg(not(feature = "proxy-transports"))]
            if refusal.reason.contains("through your proxy") {
                return ("refused: The proxy could not be reached.".to_owned(), true);
            }
            (format!("refused: {}", refusal.reason), false)
        })
}

/// Checked answers from a private page that may stay on its direct local path.
fn pinned_direct_addresses(
    destination: Destination,
    addresses: Option<&[SocketAddr]>,
) -> Option<Vec<SocketAddr>> {
    if !matches!(destination, Destination::Public | Destination::Lan) {
        return None;
    }
    let local: Vec<_> = addresses?
        .iter()
        .copied()
        .filter(|addr| matches!(classify(addr.ip()), IpClass::Private | IpClass::SharedCgnat))
        .collect();
    (!local.is_empty()).then_some(local)
}

/// The capture's media fetcher.
pub(crate) struct CoreFetch {
    shared: std::sync::Weak<Shared>,
    service: PreviewService,
    guard: Option<vw_netguard::NetGuard>,
    capture_failure: Mutex<Option<String>>,
    client: reqwest::Client,
}

impl CoreFetch {
    /// A fetcher over the session previews' policy.
    pub(crate) fn new(shared: &Arc<Shared>, service: PreviewService) -> Result<Self, String> {
        let guard = shared.preview.as_ref().and_then(|hub| hub.guard.clone());
        let client = vw_mistral::tls_builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| format!("the media fetcher could not start: {e}"))?;
        Ok(Self {
            shared: Arc::downgrade(shared),
            service,
            guard,
            capture_failure: Mutex::new(None),
            client,
        })
    }

    fn mark_capture_refusal(&self, reason: &str) -> String {
        *self
            .capture_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reason.to_owned());
        reason.to_owned()
    }

    fn guard_reason(&self, since: u64, url: &Url) -> Option<String> {
        let (reason, fatal) = guard_refusal(self.guard.as_ref()?, since, url)?;
        if fatal {
            self.mark_capture_refusal(&reason);
        }
        Some(reason)
    }

    /// Each public hop owns one client, so a pooled HTTPS connection cannot reuse an earlier PAC choice.
    fn public_client(guard: &vw_netguard::NetGuard) -> Result<reqwest::Client, String> {
        // The guard's ordinary shared listener resolves and checks the destination, then asks the selected
        // proxy for that checked IP. Only this local capability appears in reqwest's configuration.
        let proxy = reqwest::Proxy::all(guard.proxy_url())
            .map_err(|_| "the media fetcher could not set up its network guard".to_owned())?;
        vw_mistral::tls_builder()
            .redirect(reqwest::redirect::Policy::none())
            .proxy(proxy)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| "the media fetcher could not set up its network guard".to_owned())
    }

    /// A client like [`Self::new`]'s whose lookup of `url`'s host answers exactly `addrs` (checked by the caller).
    fn lan_client(url: &Url, addrs: &[SocketAddr]) -> Result<reqwest::Client, String> {
        let host = url.host_str().ok_or("refused: no host")?;
        vw_mistral::tls_builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .resolve_to_addrs(host, addrs)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| format!("the media fetcher could not start: {e}"))
    }

    async fn fetch_inner(
        &self,
        session_id: &str,
        url: &str,
        max_bytes: u64,
    ) -> Result<Fetched, String> {
        let mut current = Url::parse(url).map_err(|_| "refused: not a URL".to_owned())?;
        for _ in 0..=MAX_HOPS {
            let (destination, addresses) = self
                .service
                .fetch_allowed(session_id, &current)
                .await
                .map_err(|e| format!("refused: {e}"))?;
            if matches!(destination, Destination::Loopback { .. })
                && current.host_str() == Some("localhost")
            {
                // The resolver accepts public answers only; a session port is reached by its literal address.
                let _ = current.set_host(Some("127.0.0.1"));
            }
            // The service checks the captured page's own address space and pins every media answer.
            let direct_addresses = pinned_direct_addresses(destination, addresses.as_deref());
            if destination == Destination::Lan && direct_addresses.is_none() {
                return Err("refused: no checked local media address".to_owned());
            }
            let lan_client = direct_addresses
                .as_deref()
                .map(|addrs| Self::lan_client(&current, addrs))
                .transpose()?;
            let public = destination == Destination::Public && direct_addresses.is_none();
            let public_client = if public {
                let guard = self.guard.as_ref().ok_or_else(|| {
                    self.mark_capture_refusal("refused: the browser network guard is unavailable")
                })?;
                Some(
                    Self::public_client(guard).map_err(|reason| {
                        self.mark_capture_refusal(&format!("refused: {reason}"))
                    })?,
                )
            } else {
                None
            };
            let client = public_client
                .as_ref()
                .or(lan_client.as_ref())
                .unwrap_or(&self.client);
            let refusal_since = if public {
                self.guard.as_ref().map(vw_netguard::NetGuard::last_seq)
            } else {
                None
            };
            let mut response = client
                .get(current.clone())
                .header(reqwest::header::ACCEPT, "*/*")
                .send()
                .await
                .map_err(|e| {
                    if let Some(reason) =
                        refusal_since.and_then(|since| self.guard_reason(since, &current))
                    {
                        return reason;
                    }
                    let denied = refused_lookup(&e);
                    let text = e.without_url().to_string();
                    if denied {
                        "refused: the host resolves to a non-public address".to_owned()
                    } else {
                        format!("the request failed: {text}")
                    }
                })?;
            let status = response.status();
            if matches!(status.as_u16(), 403 | 502 | 503)
                && let Some(reason) =
                    refusal_since.and_then(|since| self.guard_reason(since, &current))
            {
                return Err(reason);
            }
            if status.is_redirection() {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| current.join(loc).ok())
                    .ok_or("a redirect without a usable address")?;
                current = next;
                continue;
            }
            if !status.is_success() {
                return Err(format!("the server answered {}", status.as_u16()));
            }
            if response.content_length().is_some_and(|n| n > max_bytes) {
                return Err(format!("larger than {max_bytes} bytes"));
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|t| t.chars().take(100).collect());
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| format!("reading failed: {}", e.without_url()))?
            {
                if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
                    return Err(format!("larger than {max_bytes} bytes"));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(Fetched {
                bytes,
                content_type,
            });
        }
        Err("too many redirects".to_owned())
    }
}

impl Fetch for CoreFetch {
    fn capture_refusal(&self) -> Option<String> {
        self.capture_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn fetch<'a>(&'a self, session_id: &'a str, url: &'a str, max_bytes: u64) -> FetchFuture<'a> {
        Box::pin(async move {
            self.fetch_inner(session_id, url, max_bytes)
                .await
                .map_err(|reason| {
                    if reason.starts_with("refused:")
                        && let Some(shared) = self.shared.upgrade()
                        && let Some(owner) = self.service.owner_session_id(session_id)
                    {
                        return crate::guard_refusals::record(
                            &shared,
                            &owner,
                            crate::GuardKind::Network,
                            crate::guard_refusals::step(&shared, &owner, None),
                            &reason,
                        );
                    }
                    reason
                })
        })
    }
}

/// Runs one capture for `session_id` by `by` with `args` (`{what, formats?, shotMethod?}`).
///
/// # Errors
/// An [`ActError`]: invalid arguments (refused), no session folder, no preview, the capture's own refusals.
pub(crate) async fn run(
    shared: &Arc<Shared>,
    session_id: &str,
    args: &Value,
    by: Actor,
) -> Result<Value, ActError> {
    let refused = |text: String| ActError {
        capture_block: None,
        protected_values_withheld: None,
        outcome: vw_browser::Outcome::Refused,
        text,
    };
    let request = capture::parse(args).map_err(refused)?;
    let hub = hub(shared).map_err(|e| refused(e.message))?;
    let owner = hub
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| refused("browser page has no owning chat".to_owned()))?;
    let folders = crate::agent_tools::session::folders(shared, &owner).map_err(refused)?;
    let folder = folders
        .first()
        .ok_or_else(|| refused("this chat has no folder".to_owned()))?;
    let fetch = CoreFetch::new(shared, hub.service.clone()).map_err(refused)?;
    hub.browser
        .capture(
            session_id,
            request,
            by,
            Target {
                session_folder: folder,
                fetch: &fetch,
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::refused_lookup;
    #[derive(Debug)]
    struct Wrapped(std::io::Error);
    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("request failed")
        }
    }
    impl std::error::Error for Wrapped {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    #[test]
    fn a_wrapped_dns_guard_refusal_is_not_mistaken_for_a_connection_failure() {
        assert!(refused_lookup(&Wrapped(std::io::Error::other(
            "refused: private address"
        ))));
        assert!(!refused_lookup(&Wrapped(std::io::Error::other(
            "connection reset"
        ))));
    }
}

#[cfg(all(test, feature = "proxy-transports"))]
#[path = "browser_capture_proxy_tests.rs"]
mod proxy_tests;
