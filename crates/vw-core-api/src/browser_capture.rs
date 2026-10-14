//! Capture in the core: the shared path of the `browser_capture` agent tool and the `browser` command's
//! `capture` op (the window's Capture menu and the Debug API), and the media fetcher.
//!
//! The fetcher ([`CoreFetch`]) gets a capture's media: only `http`/`https`, each URL (and each redirect hop, at most
//! [`MAX_HOPS`]) checked with the session's mount policy (`PreviewService::fetch_allowed`: public hosts for every
//! page; loopback only the session's own ports; the Lan page's own host), public host names resolved by
//! [`PublicOnly`], which refuses a name if ANY answer is not a public address and hands the checked addresses to the
//! connection (no second lookup, so DNS rebinding cannot swap them). No cookies, no credentials, no proxy, bounded
//! size and time.
//!
//! Depends on: vw-browser (`capture`), vw-preview (policy), vw-netguard (address classes), vw-mistral (TLS setup),
//! reqwest, `agent_tools::session` (the session folder), `previews::hub`. Called by: `browser` (command),
//! `agent_tools::browser_tools`.
//! Invariant: the capture folder is the session's first folder (`agent_tools::session::folders`); nothing is written
//! elsewhere (vw-browser `capture::files`).

use std::net::SocketAddr;
use std::sync::Arc;
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

/// The capture's media fetcher.
pub(crate) struct CoreFetch {
    service: PreviewService,
    client: reqwest::Client,
}

impl CoreFetch {
    /// A fetcher over the session previews' policy.
    pub(crate) fn new(service: PreviewService) -> Result<Self, String> {
        let client = vw_mistral::tls_builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicOnly))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| format!("the media fetcher could not start: {e}"))?;
        Ok(Self { service, client })
    }

    async fn fetch_inner(
        &self,
        session_id: &str,
        url: &str,
        max_bytes: u64,
    ) -> Result<Fetched, String> {
        let mut current = Url::parse(url).map_err(|_| "refused: not a URL".to_owned())?;
        for _ in 0..=MAX_HOPS {
            let destination = self
                .service
                .fetch_allowed(session_id, &current)
                .map_err(|e| format!("refused: {e}"))?;
            if matches!(destination, Destination::Loopback { .. })
                && current.host_str() == Some("localhost")
            {
                // The resolver accepts public answers only; a session port is reached by its literal address.
                let _ = current.set_host(Some("127.0.0.1"));
            }
            let mut response = self
                .client
                .get(current.clone())
                .header(reqwest::header::ACCEPT, "*/*")
                .send()
                .await
                .map_err(|e| {
                    let text = e.without_url().to_string();
                    if text.contains("refused:") {
                        "refused: the host resolves to a non-public address".to_owned()
                    } else {
                        format!("the request failed: {text}")
                    }
                })?;
            let status = response.status();
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
    fn fetch<'a>(&'a self, session_id: &'a str, url: &'a str, max_bytes: u64) -> FetchFuture<'a> {
        Box::pin(self.fetch_inner(session_id, url, max_bytes))
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
        outcome: vw_browser::Outcome::Refused,
        text,
    };
    let request = capture::parse(args).map_err(refused)?;
    let hub = hub(shared).map_err(|e| refused(e.message))?;
    let owner = hub
        .service
        .owner_session_id(session_id)
        .ok_or_else(|| refused("browser page has no session owner".to_owned()))?;
    let folders = crate::agent_tools::session::folders(shared, &owner).map_err(refused)?;
    let folder = folders
        .first()
        .ok_or_else(|| refused("this session has no folder".to_owned()))?;
    let fetch = CoreFetch::new(hub.service.clone()).map_err(refused)?;
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
