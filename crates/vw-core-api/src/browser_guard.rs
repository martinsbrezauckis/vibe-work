//! Live browser mount context and network proxy configuration. Backend fetches retain strict scopes.
use crate::core::Shared;
use std::sync::{Arc, Weak};
use vw_netguard::{PageContext, ScopeFuture};

/// Only a currently mounted browser page admits unrestricted browser destinations.
pub(crate) struct GuardContext {
    shared: Weak<Shared>,
}
impl GuardContext {
    pub(crate) fn new(shared: &Arc<Shared>) -> Self {
        Self {
            shared: Arc::downgrade(shared),
        }
    }
}
impl PageContext for GuardContext {
    fn browser_addresses(&self, key: &str) -> bool {
        self.shared
            .upgrade()
            .and_then(|shared| {
                shared
                    .preview
                    .as_ref()
                    .and_then(|hub| hub.service.page_for_mount(key))
            })
            .is_some()
    }
    fn loopback<'a>(&'a self, key: &'a str, _port: u16) -> ScopeFuture<'a> {
        Box::pin(async move {
            if self.browser_addresses(key) {
                Ok(())
            } else {
                Err("the browser mount is no longer active".into())
            }
        })
    }
}

/// Registers Vibe Work's own ports for strict non-browser scopes.
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

/// Updates the existing browser guard after vault-backed proxy settings become available.
pub(crate) fn refresh_network_guard(shared: &Shared) {
    let Some(guard) = shared.preview.as_ref().and_then(|hub| hub.guard.as_ref()) else {
        return;
    };
    // Public requests go through the user's proxy: Settings > Network (or the process environment) when
    // set, else the guard keeps the system's proxy settings it read when it was bound.
    #[cfg(not(feature = "proxy-transports"))]
    let network = vw_mistral::network::current();
    #[cfg(not(feature = "proxy-transports"))]
    if network.has_proxy() {
        guard.set_upstream(vw_netguard::Upstream::from_values(
            network.http_proxy.as_deref(),
            network.https_proxy.as_deref(),
            network.all_proxy.as_deref(),
            network.no_proxy.as_deref(),
        ));
    }
    #[cfg(feature = "proxy-transports")]
    {
        let network = vw_mistral::network::guard_current();
        guard.set_upstream(
            vw_netguard::Upstream::from_values(
                network.http_proxy.as_deref(),
                network.https_proxy.as_deref(),
                network.all_proxy.as_deref(),
                network.no_proxy.as_deref(),
            )
            .with_system_resolver()
            .with_certificates(network.guard_certificates()),
        );
    }
}
