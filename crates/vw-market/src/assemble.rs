//! Writing a plugin folder (Agent Plugins 1.0, the shapes Vibe's native loader accepts: `_native.py`
//! `_PluginManifest`, `_MCPConfiguration`, `_MCPStdioServer`, `_MCPStreamableHTTPServer`).
//!
//! Depends on: `catalog`, `serde_json`. Called by: `install`.
//! Invariants:
//! - Every file goes to a path the catalog validated (`catalog_rules::check_path`), joined below the staging folder.
//! - The manifest is ours (name = entry id, our description), never the vendor's: a vendor `plugin.json` could
//!   carry hooks or a reserved namespace.
//! - `${DATA}` and `${RUNTIME}` are expanded here; `${PLUGIN_ROOT}`/`${PLUGIN_DATA}` are left for Vibe.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::catalog::{Entry, McpServerSpec};
use crate::error::MarketError;

/// Files that ship inside this build and are written into plugins (our own text, MIT).
const BUNDLED: &[(&str, &str)] = &[(
    "headless-browser/SKILL.md",
    include_str!("../catalog/files/headless-browser/SKILL.md"),
)];

/// The bundled file `name`, if this build has it.
#[must_use]
pub fn bundled(name: &str) -> Option<&'static str> {
    BUNDLED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| *text)
}

/// Collects what is written, for the receipt.
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub(crate) files: BTreeMap<String, String>,
}

impl Written {
    /// Writes `bytes` to `root/rel` (folders created) and records its hash.
    pub(crate) fn write(
        &mut self,
        root: &Path,
        rel: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), MarketError> {
        crate::catalog_rules::check_path(rel)
            .map_err(|_| MarketError::Archive(format!("{rel}: bad path")))?;
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MarketError::io(format!("create the folder of {rel}"), e))?;
        }
        write_new(&path, bytes).map_err(|e| MarketError::io(format!("write {rel}"), e))?;
        if executable {
            mark_executable(&path)
                .map_err(|e| MarketError::io(format!("make {rel} executable"), e))?;
        }
        self.files
            .insert(rel.to_owned(), crate::digest::sha256_hex(bytes));
        Ok(())
    }
}

/// Creates `path` (it must not exist: `create_new`, which also never follows a link at that path) and writes `bytes`.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The manifest for `entry` at `version`.
#[must_use]
pub(crate) fn manifest(entry: &Entry, version: &str) -> Vec<u8> {
    let mut doc = Map::new();
    doc.insert(
        "$schema".into(),
        json!("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"),
    );
    doc.insert("name".into(), json!(entry.id));
    doc.insert("version".into(), json!(version));
    doc.insert("description".into(), json!(entry.summary));
    doc.insert("author".into(), json!({"name": entry.publisher}));
    if let Some(home) = &entry.homepage {
        doc.insert("homepage".into(), json!(home));
    }
    doc.insert("license".into(), json!(entry.licence));
    doc.insert("keywords".into(), json!(["vibe-work-marketplace"]));
    pretty(&Value::Object(doc))
}

/// The `mcp.json` for `servers`, with `${RUNTIME}` -> `./<runtime>` and `${DATA}` -> `data` expanded.
#[must_use]
pub(crate) fn mcp_json(
    servers: &BTreeMap<String, McpServerSpec>,
    runtime: Option<&str>,
    data: &Path,
) -> Vec<u8> {
    let data = data.to_string_lossy();
    let expand = |text: &str| {
        let text = text.replace("${DATA}", &data);
        match runtime {
            Some(runtime) => text.replace("${RUNTIME}", &format!("./{runtime}")),
            None => text,
        }
    };
    let mut out = Map::new();
    for (name, server) in servers {
        let value = match server {
            McpServerSpec::StreamableHttp { url } => json!({"type": "streamable-http", "url": url}),
            McpServerSpec::Stdio { command, args, env } => json!({
                "type": "stdio",
                "command": expand(command),
                "args": args.iter().map(|a| expand(a)).collect::<Vec<_>>(),
                "env": env.iter().map(|(k, v)| (k.clone(), Value::String(expand(v)))).collect::<Map<_, _>>(),
            }),
        };
        out.insert(name.clone(), value);
    }
    pretty(&json!({
        "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
        "mcpServers": out,
    }))
}

/// Applies `${ROOT}`/`${DATA}` to a post-install argument.
#[must_use]
pub(crate) fn expand_step(text: &str, root: &Path, data: &Path) -> String {
    text.replace("${ROOT}", &root.to_string_lossy())
        .replace("${DATA}", &data.to_string_lossy())
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_vec_pretty(value).unwrap_or_default();
    text.push(b'\n');
    text
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)] // Same signature as the Unix version; Windows has no executable bit.
fn mark_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
