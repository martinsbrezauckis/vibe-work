//! Vibe Work's MCP entries in its own Vibe config: a key server is an `mcp_servers` entry whose
//! address is the MCP forwarder on the model proxy, `http://127.0.0.1:<port>/vw-mcp/<binding id>`, with static auth
//! `api_key_env = "VW_MCP_TOKEN"` (Vibe sends the proxy token from its environment; `core/config/models.py:338-349`).
//! No key and no upstream address is ever written here.
//!
//! Why the core edits the file itself: Vibe's only method that adds a server writes OAuth entries
//! (`mcp_catalog/add`, `core/config/mcp_servers.py:51-95`), and the gate keeps `/mcp_servers` out of `config/write`
//! on purpose (any origin could then add headers or local commands). Vibe re-reads the file on `mcp_catalog/refresh`
//! (`app_server/mcp_catalog.py:274-293`) and its own writes check the file's fingerprint (`layers/_base.py:53-91`), so
//! neither side silently loses the other's change.
//!
//! Depends on: `toml_edit` (edits keep the rest of the file), `toml` (the check), vw-platform (private atomic
//! write). Called by: `mcp_keys` (add), `core` (port rewrite before the server starts).
//! Invariants:
//! - Only Vibe Work's own `VIBE_HOME/config.toml` is touched (never `~/.vibe`, same rule as `title_model`).
//! - Only entries whose address has the forwarder's shape are re-ported; every other entry is left byte-identical.
//! - Both `[[mcp_servers]]` and Vibe's inline array are edited in place; an unclassifiable row is refused.
//! - The edit is checked before it is written (the new text parses and has the expected entry), and the file is
//!   re-read right before the rename: a change in between (Vibe wrote it) is retried once, then refused.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, Value, value};

use crate::mcp_rows::edit_rows;

/// The variable Vibe reads the forwarder token from (`vw_supervisor::MCP_TOKEN_VAR`).
pub(crate) const TOKEN_ENV: &str = vw_supervisor::MCP_TOKEN_VAR;

/// Largest config file edited (Vibe's configs are a few kilobytes).
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Why an edit was not made.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigEditError {
    /// The file could not be read or written.
    #[error("Vibe's config could not be read or written: {0}")]
    Io(String),
    /// The file is not valid TOML, too large, or `mcp_servers` is not a list of tables (left alone).
    #[error("Vibe's config is not in a form Vibe Work can edit safely; nothing was changed")]
    Unsupported,
    /// A server with that name exists.
    #[error("an MCP server with that name is already configured")]
    NameTaken,
    /// The file changed twice while we edited it.
    #[error("Vibe's config changed while Vibe Work edited it; try again")]
    Busy,
    /// No command-started server of that name (a change of one that is gone, or of another kind).
    #[error("no command server with that name is configured")]
    NotACommandServer,
}

/// The forwarder address of binding `id` on the proxy at `base` (`http://127.0.0.1:<port>`).
pub(crate) fn forwarder_url(base: &str, id: &str) -> String {
    format!("{base}{}{id}", vw_keyproxy::MCP_PREFIX)
}

/// The binding id of a forwarder address on 127.0.0.1 (any port), else `None`.
pub(crate) fn forwarder_id(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("http://127.0.0.1:")?;
    let (port, path) = rest.split_once('/')?;
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id = path.strip_prefix(vw_keyproxy::MCP_PREFIX.trim_start_matches('/'))?;
    vw_vault::mcp_binding::is_binding_id(id).then_some(id)
}

pub(crate) fn read(path: &Path) -> Result<String, ConfigEditError> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_CONFIG_BYTES => Err(ConfigEditError::Unsupported),
        Ok(_) => {
            std::fs::read_to_string(path).map_err(|e| ConfigEditError::Io(e.kind().to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(ConfigEditError::Io(e.kind().to_string())),
    }
}

/// Applies `edit` to `<home>/config.toml` (see the module invariants). `edit` returns false for "nothing to do".
pub(crate) fn edit_file(
    home: &Path,
    edit: impl Fn(&mut DocumentMut) -> Result<bool, ConfigEditError>,
) -> Result<bool, ConfigEditError> {
    let path = home.join("config.toml");
    for _ in 0..2 {
        let before = read(&path)?;
        // The parser's message can quote config lines (and so a header value): never returned or logged.
        let mut doc: DocumentMut = before.parse().map_err(|_| ConfigEditError::Unsupported)?;
        if !edit(&mut doc)? {
            return Ok(false);
        }
        let after = doc.to_string();
        if after.parse::<toml::Table>().is_err() {
            return Err(ConfigEditError::Unsupported);
        }
        if read(&path)? != before {
            continue;
        }
        std::fs::create_dir_all(home).map_err(|e| ConfigEditError::Io(e.kind().to_string()))?;
        vw_platform::replace_private(&path, after.as_bytes())
            .map_err(|e| ConfigEditError::Io(e.to_string()))?;
        return Ok(true);
    }
    Err(ConfigEditError::Busy)
}

/// Adds the entry for key server `name` at forwarder address `url`.
///
/// # Errors
/// [`ConfigEditError`].
pub(crate) fn add_entry(home: &Path, name: &str, url: &str) -> Result<(), ConfigEditError> {
    edit_file(home, |doc| {
        let mut list = edit_rows(doc)?;
        if list.has_name(name)? {
            return Err(ConfigEditError::NameTaken);
        }
        let mut auth = Table::new();
        auth.insert("type", value("static"));
        auth.insert("api_key_env", value(TOKEN_ENV));
        let mut entry = Table::new();
        entry.insert("name", value(name));
        entry.insert("transport", value("streamable-http"));
        entry.insert("url", value(url));
        entry.insert("auth", Item::Table(auth));
        list.push(entry);
        Ok(true)
    })
    .map(|_| ())
}

/// Adds the entry for a server that needs no sign-in: no `auth` table, so Vibe connects without a credential
/// and never asks the server for a sign-in.
///
/// # Errors
/// [`ConfigEditError`].
pub(crate) fn add_unauthenticated_entry(
    home: &Path,
    name: &str,
    url: &str,
) -> Result<(), ConfigEditError> {
    edit_file(home, |doc| {
        let mut list = edit_rows(doc)?;
        if list.has_name(name)? {
            return Err(ConfigEditError::NameTaken);
        }
        let mut entry = Table::new();
        entry.insert("name", value(name));
        entry.insert("transport", value("streamable-http"));
        entry.insert("url", value(url));
        list.push(entry);
        Ok(true)
    })
    .map(|_| ())
}

/// Points every forwarder entry at the proxy at `base` (`http://127.0.0.1:<port>`): called before the server
/// starts, because the proxy's port is new on every app start. Returns how many entries changed.
///
/// # Errors
/// [`ConfigEditError`] (the caller logs it; the servers then answer "unreachable" until the next start).
pub(crate) fn rewrite_ports(home: &Path, base: &str) -> Result<usize, ConfigEditError> {
    let path = home.join("config.toml");
    if !path.is_file() {
        return Ok(0);
    }
    let changed = std::sync::atomic::AtomicUsize::new(0);
    edit_file(home, |doc| {
        changed.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut list = edit_rows(doc)?;
        list.for_each(|mut row| {
            let Some(id) = row.get_str("url").and_then(forwarder_id).map(str::to_owned) else {
                return Ok(false);
            };
            let url = forwarder_url(base, &id);
            if row.get_str("url") != Some(url.as_str()) {
                row.set("url", Value::from(url));
                changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(true);
            }
            Ok(false)
        })
    })?;
    Ok(changed.load(std::sync::atomic::Ordering::Relaxed))
}

/// Adds an MCP server brought from another agent's setup (no key: a program with its arguments and environment,
/// or an address with plain headers).
///
/// # Errors
/// [`ConfigEditError`] (`NameTaken` when a server of that name exists).
pub(crate) fn add_imported(
    home: &Path,
    name: &str,
    server: &vw_import::mcp::Server,
    paused: bool,
) -> Result<(), ConfigEditError> {
    edit_file(home, |doc| {
        let mut list = edit_rows(doc)?;
        if list.has_name(name)? {
            return Err(ConfigEditError::NameTaken);
        }
        let pairs = |items: &[(String, String)]| {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in items {
                table.insert(k, v.as_str().into());
            }
            toml_edit::Value::InlineTable(table)
        };
        let mut entry = Table::new();
        entry.insert("name", value(name));
        match server {
            vw_import::mcp::Server::Stdio { command, args, env } => {
                entry.insert("transport", value("stdio"));
                entry.insert("command", value(command.as_str()));
                if paused {
                    entry.insert("disabled", value(true));
                }
                let mut list = toml_edit::Array::new();
                for arg in args {
                    list.push(arg.as_str());
                }
                entry.insert("args", Item::Value(toml_edit::Value::Array(list)));
                if !env.is_empty() {
                    entry.insert("env", Item::Value(pairs(env)));
                }
            }
            vw_import::mcp::Server::Http { url, headers } => {
                entry.insert("transport", value("streamable-http"));
                entry.insert("url", value(url.as_str()));
                if !headers.is_empty() {
                    let mut auth = Table::new();
                    auth.insert("type", value("static"));
                    auth.insert("headers", Item::Value(pairs(headers)));
                    entry.insert("auth", Item::Table(auth));
                }
            }
        }
        list.push(entry);
        Ok(true)
    })
    .map(|_| ())
}

#[cfg(test)]
#[path = "mcp_keys_config_tests.rs"]
mod tests;
