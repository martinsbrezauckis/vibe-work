//! The MCP servers in Vibe Work's own Vibe config: Vibe has no sessionless
//! list of servers (`mcp_catalog/read` needs a session, `config/read` only counts them), so the Connectors
//! page reads the `mcp_servers` list of `<own VIBE_HOME>/config.toml` itself, read-only.
//!
//! Depends on: `toml` (parsing), `url` (address without query), `connectors_view`. Called by: `connectors`.
//! Invariants:
//! - Only Vibe Work's own home is read, never `~/.vibe`, and nothing is ever written.
//! - Nothing that can hold a secret leaves this module: no headers, no `api_key_env`, no environment of a
//!   local server, no command line, no query string or fragment of an address. What is kept: the name, the
//!   transport, the address as scheme + host + port + path, the kind of sign-in, the off flag. Every
//!   credential-like value read is also registered as a stored secret (`vw_secrets`).
//! - A missing, unreadable or malformed file is "no servers" (logged), never an error: Vibe itself reports
//!   a broken config at startup.

use std::path::Path;

use tracing::warn;

use crate::connectors_view::ServerView;

/// Largest config file read (Vibe's configs are a few kilobytes).
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Vibe's legacy static-auth keys (`core/config/models.py:240`): a server with one of them signs in with a
/// header or key from the config, not OAuth.
const LEGACY_STATIC_KEYS: &[&str] = &["headers", "api_key_env", "api_key_header", "api_key_format"];

/// `scheme://host[:port]/path` of `text`, without user info, query or fragment; `None` when not a URL.
#[must_use]
pub(crate) fn public_address(text: &str) -> Option<String> {
    let url = url::Url::parse(text).ok()?;
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{}://{host}{port}{}", url.scheme(), url.path()))
}

fn sign_in_kind(server: &toml::Table) -> &'static str {
    if let Some(auth) = server.get("auth").and_then(toml::Value::as_table) {
        return match auth.get("type").and_then(toml::Value::as_str) {
            Some("oauth") => "oauth",
            Some("static") => "static",
            _ => "unknown",
        };
    }
    if LEGACY_STATIC_KEYS.iter().any(|k| server.contains_key(*k)) {
        "static"
    } else {
        "none"
    }
}

/// Registers every credential-like value of one server entry as a stored secret: header values,
/// environment values, string values of its `auth` table, and the user info of its address. The Debug API's
/// response guard then replaces them wherever they would appear (a transcript, a terminal, a Vibe answer).
fn register_credentials(server: &toml::Table) {
    let register = |value: &str| {
        vw_secrets::register(vw_secrets::SecretKind::ConnectorCredential, value);
        // `Bearer <token>`, `Basic <token>`: the token alone is the secret too (a user message, a log line).
        if let Some((_, last)) = value.trim().rsplit_once(char::is_whitespace) {
            vw_secrets::register(vw_secrets::SecretKind::ConnectorCredential, last);
        }
    };
    for key in ["headers", "env", "auth"] {
        if let Some(table) = server.get(key).and_then(toml::Value::as_table) {
            for value in table.values().filter_map(toml::Value::as_str) {
                register(value);
            }
        }
    }
    if let Some(url) = server
        .get("url")
        .and_then(toml::Value::as_str)
        .and_then(|u| url::Url::parse(u).ok())
    {
        if let Some(password) = url.password() {
            register(password);
        }
        if !url.username().is_empty() {
            register(url.username());
        }
    }
}

fn server_view(server: &toml::Table) -> Option<ServerView> {
    register_credentials(server);
    let name = server.get("name")?.as_str()?.to_owned();
    let transport = server
        .get("transport")
        .and_then(toml::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let address = server
        .get("url")
        .and_then(toml::Value::as_str)
        .and_then(public_address);
    Some(ServerView {
        display_name: name.clone(),
        name,
        transport,
        address,
        sign_in: sign_in_kind(server).to_owned(),
        off: server.get("disabled").and_then(toml::Value::as_bool) == Some(true),
        status: None,
        tools: Vec::new(),
        error: None,
        plugin: None,
        configured: true,
        key: None,
    })
}

/// Registers the credentials of the servers in `<home>/config.toml` as stored secrets, at core
/// start: before any session can echo them, the Debug API's response guard already replaces them.
pub(crate) fn register_at_start(home: &Path) {
    let _ = configured_servers(home);
}

/// The servers configured in `<home>/config.toml`, in file order (see the module docs).
#[must_use]
pub(crate) fn configured_servers(home: &Path) -> Vec<ServerView> {
    let path = home.join("config.toml");
    let text = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= MAX_CONFIG_BYTES => match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                warn!(%error, "connectors: Vibe's config could not be read");
                return Vec::new();
            }
        },
        Ok(_) => {
            warn!("connectors: Vibe's config is larger than 1 MiB; servers not listed");
            return Vec::new();
        }
        Err(_) => return Vec::new(),
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        // The parser's message can quote config lines (and so a header value): not logged.
        warn!("connectors: Vibe's config is not valid TOML; servers not listed");
        return Vec::new();
    };
    table
        .get("mcp_servers")
        .and_then(toml::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(toml::Value::as_table)
                .filter_map(server_view)
                .collect()
        })
        .unwrap_or_default()
}
