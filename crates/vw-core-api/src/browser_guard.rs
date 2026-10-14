//! The core's side of the agent browser's network guard:
//! answers, for one webview mount, whether a request may reach a port on this computer, and keeps the guard's list of
//! Vibe Work's own ports current (the agent-tools endpoint, the key proxy, the development origin; the host adds the
//! Debug API).
//!
//! Depends on: vw-netguard (`PageContext`), vw-preview (`loopback_gate`), `preview_ports` (owners), `core::Shared`.
//! Called by: `previews` (start, shutdown, session end), the guard (per loopback request), vw-host (Debug API port).
//! Invariants:
//! - A mount holding a page from the web reaches nothing on this computer.
//! - A mount holding a local-network page the user opened reaches that page's own host only (`lan_host`).
//! - A mount holding a page of the session's own server reaches its session's ports (attributed or leased) and other
//!   sessions' attributed ports (background requests: a backend in one session, frontends in others); never
//!   another app's port or a free one.
//! - The app-wide listener (`WebView2`) asks about every live mount ([`PageContext::mounts`]) and passes
//!   what any of them may reach; each webview's own request barrier keeps it to its mount's rights.
//! - Owners are read from the process monitor (a scan) at most every [`OWNER_TTL`] per session, so lease and process
//!   changes apply within that time; scans run on a blocking thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use vw_netguard::{PageContext, ScopeFuture};
use vw_preview::{LoopbackGate, PortOwner};

use crate::core::Shared;
use crate::preview_ports::{SessionPorts, port_owner, session_ports};

/// How long a session's port owners are reused.
pub(crate) const OWNER_TTL: Duration = Duration::from_secs(2);

/// The page context the guard asks.
pub(crate) struct GuardContext {
    shared: Weak<Shared>,
    cache: Mutex<HashMap<String, (Instant, Arc<SessionPorts>)>>,
}

impl GuardContext {
    pub(crate) fn new(shared: &Arc<Shared>) -> Self {
        Self {
            shared: Arc::downgrade(shared),
            cache: Mutex::new(HashMap::new()),
        }
    }

    async fn ports(
        &self,
        shared: &Arc<Shared>,
        session: &str,
    ) -> Result<Arc<SessionPorts>, String> {
        if let Some((at, ports)) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            && at.elapsed() < OWNER_TTL
        {
            return Ok(ports.clone());
        }
        let (owned, id) = (shared.clone(), session.to_owned());
        let ports = tokio::task::spawn_blocking(move || session_ports(&owned, &id))
            .await
            .map_err(|e| format!("the process monitor failed: {e}"))??;
        let ports = Arc::new(ports);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session.to_owned(), (Instant::now(), ports.clone()));
        Ok(ports)
    }
}

impl PageContext for GuardContext {
    fn loopback<'a>(&'a self, key: &'a str, port: u16) -> ScopeFuture<'a> {
        Box::pin(async move {
            let shared = self
                .shared
                .upgrade()
                .ok_or_else(|| "Vibe Work is closing".to_owned())?;
            let hub = shared
                .preview
                .clone()
                .ok_or_else(|| "the browser is not available".to_owned())?;
            let session = match hub.service.loopback_gate(key) {
                LoopbackGate::NeedOwner(session) => session,
                LoopbackGate::Refuse(reason) => return Err(reason),
            };
            let ports = self.ports(&shared, &session).await?;
            match port_owner(&shared, &ports, &session, port, false) {
                PortOwner::ThisSession | PortOwner::OtherSession(_) => Ok(()),
                PortOwner::OtherApp(owner) => Err(format!("port {port} belongs to {owner}")),
                PortOwner::Nobody | PortOwner::NotProven => Err(format!(
                    "port {port} is not one of this session's servers or another session's server"
                )),
            }
        })
    }

    fn mounts(&self) -> Vec<String> {
        self.shared
            .upgrade()
            .and_then(|shared| {
                shared
                    .preview
                    .as_ref()
                    .map(|hub| hub.service.mount_labels())
            })
            .unwrap_or_default()
    }

    fn lan_host(&self, key: &str) -> Option<String> {
        self.shared
            .upgrade()?
            .preview
            .as_ref()?
            .service
            .lan_host_of(key)
    }
}

/// Registers Vibe Work's own ports the core knows with the guard (never reachable through it).
pub(crate) fn register_internal_ports(shared: &Shared, guard: &vw_netguard::NetGuard) {
    if let Some(tools) = &shared.agent_tools {
        guard.add_internal_port(tools.endpoint.port());
    }
    if let Some(keys) = &shared.config.keys
        && let Some(port) = url::Url::parse(keys.proxy_base())
            .ok()
            .and_then(|u| u.port_or_known_default())
    {
        guard.add_internal_port(port);
    }
    if let Some(hub) = &shared.preview
        && let Some(port) = hub
            .service
            .policy()
            .dev_origin
            .as_ref()
            .and_then(url::Url::port_or_known_default)
    {
        guard.add_internal_port(port);
    }
}
