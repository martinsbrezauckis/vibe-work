//! Per-request vault-key resolution and forwarder lifecycle for MCP servers.
//! Depends on the parent's checked key path (`mcp_keys_gate`), grant counters and key proxy; called at core start/stop.
//! Values stay in the outgoing request. Every request reads the key only through the gate, which checks current
//! authority and binding state before and after the read. A request to a server without a grant may first wait for
//! the person's answer to the first-use dialog (`mcp_keys_first_use`).
use crate::core::Shared;
use std::sync::{Arc, Weak};
use vw_keyproxy::{McpFuture, McpRefusal, McpResolver, McpTarget};
use vw_keys::KeyService;
use zeroize::Zeroizing;

/// The resolver the proxy's MCP forwarder asks (module docs). Holds the core weakly: the key service outlives it.
struct Resolver {
    shared: Weak<Shared>,
}

impl McpResolver for Resolver {
    fn resolve<'a>(&'a self, id: &'a str) -> McpFuture<'a> {
        Box::pin(async move {
            let shared = self.shared.upgrade().ok_or(McpRefusal::Unknown)?;
            let Some(keys) = shared.config.keys.as_ref() else {
                return Err(McpRefusal::Unknown);
            };
            let paused = || crate::agent_access::locked(&shared);
            let gate = super::gate::Gate {
                vault: keys.as_ref(),
                grants: &shared.connectors.mcp_keys,
                paused: &paused,
                id,
            };
            // No grant yet: a never-revoked server asks the person once per run (`first_use`); the gate asks only
            // after its Never checks, so a User only key never opens a dialog, and checks the answer on a new read.
            let shared_ref = &shared;
            let (binding, key) = gate
                .open(move |shown| async move { super::first_use::ask(shared_ref, &shown).await })
                .await?;
            let value = if binding.bearer {
                Zeroizing::new(format!("Bearer {}", key.expose()))
            } else {
                Zeroizing::new(key.expose().to_owned())
            };
            shared.connectors.mcp_keys.count(&binding.id);
            Ok(McpTarget {
                upstream: binding.upstream,
                header: binding.header,
                value,
            })
        })
    }
}

/// Before the server starts: points Vibe Work's forwarder entries at this run's proxy port (`mcp_keys_config`).
pub(crate) fn prepare_config(home: &std::path::Path, keys: Option<&Arc<KeyService>>) {
    let Some(keys) = keys else {
        return;
    };
    match crate::mcp_keys_config::rewrite_ports(home, keys.proxy_base()) {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            entries = n,
            "mcp key servers: forwarder port updated in Vibe's config"
        ),
        Err(error) => tracing::warn!(%error, "mcp key servers: Vibe's config not updated"),
    }
}

/// Core start: registers the credentials of Vibe Work's own Vibe config as stored secrets and installs
/// the resolver in the key service's proxy.
pub(crate) fn at_start(shared: &Arc<Shared>) {
    crate::connectors_config::register_at_start(&shared.instructions.own_home);
    install(shared);
}

/// Installs the resolver in the key service's proxy.
fn install(shared: &Arc<Shared>) {
    if let Some(keys) = shared.config.keys.as_ref() {
        keys.set_mcp_resolver(Arc::new(Resolver {
            shared: Arc::downgrade(shared),
        }));
    }
}

/// Removes the resolver (core shutdown): forwarded requests answer 503 until a core installs one again.
pub(crate) fn uninstall(shared: &Shared) {
    if let Some(keys) = shared.config.keys.as_ref() {
        keys.clear_mcp_resolver();
    }
}
