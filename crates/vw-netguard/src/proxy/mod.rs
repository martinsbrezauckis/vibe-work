//! Optional system/PAC proxy transports. Called only after decide has checked every destination address.
//! Depends on the OS resolver in vw-platform, hyper-util matching and TLS/SOCKS transports. Settings win;
//! only an explicit DIRECT permits direct fallback. No destination hostname is sent in a proxy handshake.
mod error;
mod route;
mod socks;
mod transport;

use crate::stream::Stream;
pub use error::Error;
use hyper_util::client::proxy::matcher::Matcher;
pub use route::pac_routes;
pub(crate) use route::{Kind, Route};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// The OS-owned PAC/system lookup. Receives only destinations already accepted by the guard.
/// Return proxy URLs in fallback order; `direct://` is the sole direct spelling. Never evaluate JS in the guard.
pub trait ProxyResolver: Send + Sync {
    /// Resolve an allowed URL. Errors must not contain configuration or credentials.
    ///
    /// # Errors
    /// A fixed failure when discovery cannot produce a trustworthy list.
    fn resolve(&self, url: &str) -> Result<Vec<String>, Error>;
}
static LOOKUPS: std::sync::OnceLock<Arc<Semaphore>> = std::sync::OnceLock::new();
struct System;
impl ProxyResolver for System {
    fn resolve(&self, url: &str) -> Result<Vec<String>, Error> {
        vw_platform::proxy::resolve(url).map_err(|_| Error::Discovery)
    }
}

/// Upstream settings; Debug never exposes endpoints or credentials.
pub struct Upstream {
    http: Option<String>,
    https: Option<String>,
    all: Option<String>,
    bypass: Matcher,
    resolver: Option<Arc<dyn ProxyResolver>>,
    roots: Vec<Vec<u8>>,
    proxy_roots: std::collections::BTreeMap<String, Vec<Vec<u8>>>,
}
impl std::fmt::Debug for Upstream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upstream").finish_non_exhaustive()
    }
}
impl Upstream {
    /// Explicit Settings values. Unset schemes connect directly unless a system resolver is supplied.
    #[must_use]
    pub fn from_values(
        http: Option<&str>,
        https: Option<&str>,
        all: Option<&str>,
        no: Option<&str>,
    ) -> Self {
        let value = |v: Option<&str>| {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Self {
            http: value(http),
            https: value(https),
            all: value(all),
            bypass: Matcher::builder()
                .all("http://bypass.invalid")
                .no(no.unwrap_or_default())
                .build(),
            resolver: None,
            roots: Vec::new(),
            proxy_roots: std::collections::BTreeMap::new(),
        }
    }
    /// Process proxy environment first, then native OS configuration/PAC for unconfigured schemes.
    #[must_use]
    pub fn system() -> Self {
        let env = |key: &str| {
            std::env::var(key.to_ascii_lowercase())
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| std::env::var(key).ok())
        };
        Self::from_values(
            env("HTTP_PROXY").as_deref(),
            env("HTTPS_PROXY").as_deref(),
            env("ALL_PROXY").as_deref(),
            env("NO_PROXY").as_deref(),
        )
        .with_resolver(Arc::new(System))
    }
    /// No proxy; does not consult the environment or desktop.
    #[must_use]
    pub fn none() -> Self {
        Self::from_values(None, None, None, None)
    }
    /// Native resolver (or a deterministic test stub), consulted only when Settings have no applicable proxy.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn ProxyResolver>) -> Self {
        self.resolver = Some(resolver);
        self
    }
    /// Use the native resolver for schemes with no Settings proxy.
    #[must_use]
    pub fn with_system_resolver(self) -> Self {
        self.with_resolver(Arc::new(System))
    }
    /// Extra trusted DER certificates from verified Settings snapshots; never rereads saved paths.
    #[must_use]
    pub fn with_certificates(mut self, roots: Vec<Vec<u8>>) -> Self {
        self.roots = roots;
        self
    }

    /// TLS roots bound to an individual configured proxy credential. These replace the shared extra roots for that route.
    #[must_use]
    pub fn with_proxy_certificates(
        mut self,
        roots: std::collections::BTreeMap<String, Vec<Vec<u8>>>,
    ) -> Self {
        self.proxy_roots = roots;
        self
    }

    async fn routes(&self, url: &url::Url) -> Result<Vec<Option<Route>>, Error> {
        let uri = url.as_str().parse().map_err(|_| Error::Configuration)?;
        if self.bypass.intercept(&uri).is_none() {
            return Ok(vec![None]);
        }
        let configured = if url.scheme() == "https" {
            &self.https
        } else {
            &self.http
        };
        if let Some(raw) = configured.as_ref().or(self.all.as_ref()) {
            // Validate before matching: an unsupported/malformed configured proxy is never silently DIRECT.
            let mut route = Route::parse(raw)?;
            let key = if configured.is_some() {
                if url.scheme() == "https" {
                    "HTTPS_PROXY"
                } else {
                    "HTTP_PROXY"
                }
            } else {
                "ALL_PROXY"
            };
            route.roots = self.proxy_roots.get(key).cloned();
            return Ok(vec![Some(route)]);
        }
        let Some(resolver) = &self.resolver else {
            return Ok(vec![None]);
        };
        // Existing guard connection slots bound the async waiters. A normal page can need more than four
        // lookups, so wait within the same deadline instead of rejecting its fifth connection. Timed-out native
        // work retains its permit; repeated bad PAC requests cannot grow an unbounded blocking queue.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let slots = LOOKUPS.get_or_init(|| Arc::new(Semaphore::new(4))).clone();
        let permit = tokio::time::timeout_at(deadline, slots.acquire_owned())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Timeout)?;
        let resolver = resolver.clone();
        let mut target = url.clone();
        let _ = target.set_username("");
        let _ = target.set_password(None);
        target.set_fragment(None);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // A native library may not support cancellation. Detached, bounded OS workers let runtime shutdown
        // finish even if a bad PAC never returns; the worker keeps its permit until it actually exits.
        std::thread::Builder::new()
            .name("system-proxy".to_owned())
            .spawn(move || {
                let _permit = permit;
                let _ = sender.send(resolver.resolve(target.as_str()));
            })
            .map_err(|_| Error::Discovery)?;
        let values = tokio::time::timeout_at(deadline, receiver)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Discovery)??;
        if values.is_empty() || values.len() > 32 {
            return Err(Error::Configuration);
        }
        values
            .iter()
            .map(|v| {
                if v == "direct://" {
                    Ok(None)
                } else {
                    Route::parse(v).map(Some)
                }
            })
            .collect()
    }

    pub(crate) async fn open(
        &self,
        url: &url::Url,
        tunnel: bool,
        checked: &[SocketAddr],
    ) -> Result<(Stream, Option<Route>), Error> {
        let routes = self.routes(url).await?;
        let fallback_list = routes.len() > 1;
        let mut last = Error::Unreachable;
        for route in routes {
            for addr in checked {
                let result = tokio::time::timeout(Duration::from_secs(15), async {
                    if let Some(proxy) = &route {
                        // Keep the existing HTTP absolute-form path for a single manual HTTP proxy.
                        let absolute = !tunnel && !fallback_list && proxy.kind == Kind::Http;
                        let stream = transport::open(
                            proxy,
                            *addr,
                            proxy.roots.as_deref().unwrap_or(&self.roots),
                            !absolute,
                        )
                        .await?;
                        Ok((stream, if absolute { route.clone() } else { None }))
                    } else {
                        tokio::net::TcpStream::connect(addr)
                            .await
                            .map(|s| (Box::new(s) as Stream, None))
                            .map_err(|_| Error::Unreachable)
                    }
                })
                .await
                .unwrap_or(Err(Error::Timeout));
                match result {
                    Ok(connection) => return Ok(connection),
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    }
}

/// Whether a connected proxy is on this computer; diagnostic only, never affects destination policy.
#[must_use]
pub fn on_this_computer(local: SocketAddr, peer: SocketAddr) -> bool {
    let ip = peer.ip();
    ip.is_loopback() || ip.is_unspecified() || ip == local.ip()
}
/// Replace an absolute HTTP target with its checked IP, retaining the original Host separately.
pub(crate) fn to_address(url: &url::Url, to: SocketAddr) -> Result<url::Url, String> {
    let mut out = url.clone();
    out.set_ip_host(to.ip())
        .map_err(|()| "The checked address is not valid.".to_owned())?;
    out.set_port(Some(to.port()))
        .map_err(|()| "The checked port is not valid.".to_owned())?;
    Ok(out)
}

#[cfg(test)]
mod tests;
