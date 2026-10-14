//! Turns Vibe's connector and MCP replies into the views of `connectors_view`.
//!
//! Depends on: `serde_json`, `connectors_view`, `connectors_config::public_address`. Called by:
//! `connectors`.
//! Response shapes from Vibe's `app_server/protocol.py`:
//! - `connector_catalog/read` -> `{catalog: {disposition, catalogRevision, connectors: [{alias, displayName,
//!   readiness, authAction, tools: [{name, description}], diagnostic}]}, selections: [{alias, disabled,
//!   disabledTools, state}], session: {sources: [{alias, displayName, status, tools: [{name, description,
//!   enabled}], error}]} | null, manageUrl}`;
//! - `mcp_catalog/read` -> `{mcp: {sources: [{name, displayName, kind: server|connector, transport, status,
//!   tools: [{name, description, enabled}], error, pluginName}], ...}}`.
//!
//! Invariant: unknown or missing fields give empty values, never a failure (a newer Vibe may add fields); an
//! address is reduced to scheme + host + port + path.

use serde_json::Value;

use crate::connectors_config::public_address;
use crate::connectors_view::{CatalogLoad, ConnectorView, ServerView, ToolView};

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn tools(value: &Value) -> Vec<ToolView> {
    list(value, "tools")
        .iter()
        .filter_map(|tool| {
            Some(ToolView {
                name: text(tool, "name")?,
                description: text(tool, "description").filter(|d| !d.is_empty()),
                enabled: tool.get("enabled").and_then(Value::as_bool),
            })
        })
        .collect()
}

/// Mistral's console, where a connector's account link is managed.
const CONSOLE: &str = "https://console.mistral.ai";

/// The "manage connectors" link Vibe built, made openable in a browser, or `None`.
///
/// Vibe builds it from `console_base_url` (`connector_catalog.py:1527-1559`), which Vibe Work points at its
/// own key proxy (`http://127.0.0.1:<port>/console`): taken as is, the browser would open the
/// proxy, which refuses it. A loopback address whose path starts with
/// `/console/` is mapped back to the console; an `https://console.mistral.ai` link is kept; anything
/// else is dropped (the page then says where to go instead of offering a link).
pub(crate) fn manage_link(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let tail = |path: &str| {
        let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
        format!("{CONSOLE}{path}{query}")
    };
    if loopback && url.scheme() == "http" {
        return url
            .path()
            .strip_prefix("/console")
            .filter(|p| p.starts_with('/'))
            .map(tail);
    }
    if url.scheme() == "https" && url.host_str() == Some("console.mistral.ai") {
        return Some(url.as_str().to_owned());
    }
    None
}

/// The catalog's freshness from Vibe's `disposition`.
pub(crate) fn catalog_load(read: &Value) -> CatalogLoad {
    match read.pointer("/catalog/disposition").and_then(Value::as_str) {
        Some("memory") => CatalogLoad::Fresh,
        Some("fresh_cache") => CatalogLoad::Cached,
        _ => CatalogLoad::NotLoaded,
    }
}

/// The connectors of a `connector_catalog/read` result, with the off flags of its selections.
pub(crate) fn connectors(read: &Value) -> Vec<ConnectorView> {
    let off = |alias: &str| {
        list(read, "selections").iter().any(|s| {
            s.get("alias").and_then(Value::as_str) == Some(alias)
                && s.get("disabled").and_then(Value::as_bool) == Some(true)
        })
    };
    let Some(catalog) = read.get("catalog") else {
        return Vec::new();
    };
    list(catalog, "connectors")
        .iter()
        .filter_map(|entry| {
            let alias = text(entry, "alias")?;
            Some(ConnectorView {
                display_name: text(entry, "displayName").unwrap_or_else(|| alias.clone()),
                readiness: text(entry, "readiness").unwrap_or_else(|| "unavailable".to_owned()),
                auth_action: text(entry, "authAction").unwrap_or_else(|| "unknown".to_owned()),
                off: off(&alias),
                tools: tools(entry),
                diagnostic: text(entry, "diagnostic").filter(|d| !d.is_empty()),
                session_status: None,
                alias,
            })
        })
        .collect()
}

/// Folds a session's view of the connectors (`session.sources` of a session-targeted read) into
/// `connectors`: status, and the session's tool list when it knows more tools.
pub(crate) fn merge_session_connectors(connectors: &mut [ConnectorView], read: &Value) {
    let Some(session) = read.get("session") else {
        return;
    };
    for source in list(session, "sources") {
        let Some(alias) = source.get("alias").and_then(Value::as_str) else {
            continue;
        };
        if let Some(connector) = connectors.iter_mut().find(|c| c.alias == alias) {
            connector.session_status = text(source, "status");
            let live = tools(source);
            if live.len() >= connector.tools.len() {
                connector.tools = live;
            }
            if connector.diagnostic.is_none() {
                connector.diagnostic = text(source, "error").filter(|e| !e.is_empty());
            }
        }
    }
}

/// Folds a session's `mcp_catalog/read` into `servers` (the configured ones): status, tools and errors
/// for known names; servers only the session knows (plugins, trusted project configs) are appended.
pub(crate) fn merge_session_servers(servers: &mut Vec<ServerView>, read: &Value) {
    let Some(mcp) = read.get("mcp") else {
        return;
    };
    for source in list(mcp, "sources") {
        if source.get("kind").and_then(Value::as_str) == Some("connector") {
            continue;
        }
        let Some(name) = text(source, "name") else {
            continue;
        };
        let status = text(source, "status");
        let error = text(source, "error").filter(|e| !e.is_empty());
        let plugin = text(source, "pluginName");
        if let Some(server) = servers.iter_mut().find(|s| s.name == name) {
            server.status = status;
            server.tools = tools(source);
            server.error = error;
            server.plugin = plugin;
            if let Some(display) = text(source, "displayName").filter(|d| !d.is_empty()) {
                server.display_name = display;
            }
            continue;
        }
        servers.push(ServerView {
            display_name: text(source, "displayName")
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| name.clone()),
            transport: text(source, "transport").unwrap_or_else(|| "unknown".to_owned()),
            address: text(source, "url").and_then(|u| public_address(&u)),
            sign_in: "unknown".to_owned(),
            off: false,
            status,
            tools: tools(source),
            error,
            plugin,
            configured: false,
            key: None,
            name,
        });
    }
}
