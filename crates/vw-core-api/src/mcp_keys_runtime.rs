//! Per-request vault-key resolution and forwarder lifecycle for MCP servers.
//! Depends on the parent's grant counters, current vault records and key proxy; called at core start/stop.
//! Values stay in the outgoing request and every request checks current authority and binding state.
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
            if crate::agent_access::locked(&shared) {
                return Err(McpRefusal::AgentAccessLocked);
            }
            let Some(keys) = shared.config.keys.as_ref() else {
                return Err(McpRefusal::Unknown);
            };
            // The vault's current record, read for this request (never a copy): a record revoked or removed by
            // another window, another app on the same vault or a restore is seen at once. Any read failure refuses.
            let binding = match keys.mcp_binding(id.to_owned()).await {
                Ok(Some(binding)) => binding,
                Ok(None) => return Err(McpRefusal::Unknown),
                Err(e) if vw_keys::is_locked(&e) => return Err(McpRefusal::Locked),
                Err(_) => return Err(McpRefusal::NotAllowed),
            };
            // This connection is shared across chats, so even an existing Always grant cannot prove Full access.
            if binding.secret_id == vw_keys::DEBUG_TOKEN_ITEM {
                return Err(McpRefusal::DebugTokenRequiresChat);
            }
            if shared.connectors.mcp_keys.grant_word(&binding) == "none" {
                return Err(McpRefusal::NotAllowed);
            }
            // The MCP proxy has no originating session/project. Never is conservative across project records;
            // Allowed cannot be promoted to a global grant and still needs this server's existing grant.
            if keys
                .default_entry_permission(&binding.secret_id)
                .await
                .map_err(|_| McpRefusal::PermissionsUnreadable)?
                .0
                == vw_keys::EntryPermission::Never
                || keys
                    .entry_permissions_for(&binding.secret_id)
                    .await
                    .map_err(|_| McpRefusal::PermissionsUnreadable)?
                    .iter()
                    .any(|p| p.permission == vw_keys::EntryPermission::Never)
            {
                return Err(McpRefusal::NotAllowed);
            }
            let key = keys
                .secret_value(binding.secret_id.clone(), None)
                .await
                .map_err(|e| {
                    if vw_keys::is_locked(&e) {
                        McpRefusal::Locked
                    } else {
                        McpRefusal::SecretMissing
                    }
                })?;
            if keys
                .default_entry_permission(&binding.secret_id)
                .await
                .map_err(|_| McpRefusal::PermissionsUnreadable)?
                .0
                == vw_keys::EntryPermission::Never
                || keys
                    .entry_permissions_for(&binding.secret_id)
                    .await
                    .map_err(|_| McpRefusal::PermissionsUnreadable)?
                    .iter()
                    .any(|p| p.permission == vw_keys::EntryPermission::Never)
            {
                return Err(McpRefusal::NotAllowed);
            }
            // Re-read after every awaited secret/policy check: the earlier binding cannot prove that a grant
            // still holds. A changed destination or secret also refuses this request rather than using stale data.
            let current = match keys.mcp_binding(id.to_owned()).await {
                Ok(Some(binding)) => binding,
                Ok(None) => return Err(McpRefusal::Unknown),
                Err(e) if vw_keys::is_locked(&e) => return Err(McpRefusal::Locked),
                Err(_) => return Err(McpRefusal::NotAllowed),
            };
            if current != binding || !shared.connectors.mcp_keys.permit(&current) {
                return Err(McpRefusal::NotAllowed);
            }
            if crate::agent_access::locked(&shared) {
                return Err(McpRefusal::AgentAccessLocked);
            }
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
