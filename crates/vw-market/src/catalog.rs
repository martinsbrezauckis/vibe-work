//! The catalog: what the Tools marketplace offers, as one JSON document (`catalog/index.json`, compiled in).
//!
//! Three kinds of entries:
//! - [`Kind::Pack`]: a skill pack = a Vibe plugin we assemble from a vendor's files at a full commit, each file pinned
//!   by SHA-256, plus optionally an `mcp.json` for a server that needs no sign-in;
//! - [`Kind::Server`]: a remote MCP server with its measured access class (Vibe signs in, or the vendor allows
//!   approved apps only, or needs your own app, or a key);
//! - [`Kind::Plugin`]: a local plugin that runs code: a runtime pinned per OS, npm packages pinned by SRI, an optional
//!   post-install step, an `mcp.json` template and our own skill text.
//!
//! Depends on: serde. Called by: `install`, `store`, vw-core-api (the views).
//! Invariants:
//! - [`Catalog::validate`] is the only way a catalog is accepted: every hash is in its full form, every path is
//!   relative without `..`, every address is `https://` (loopback `http://` only for a test index), ids are unique and
//!   usable as Vibe plugin names. A catalog that fails is refused whole.
//! - Nothing in this module touches the network or the disk.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::MarketError;

/// The built-in catalog, reviewed in this repository (`crates/vw-market/catalog/index.json`).
pub const BUILT_IN: &str = include_str!("../catalog/index.json");

/// The platforms a `plugin` entry of the built-in catalog must cover (parity: same features on every OS).
pub const REQUIRED_TARGETS: &[&str] = &[
    "linux-x64",
    "linux-arm64",
    "darwin-x64",
    "darwin-arm64",
    "win-x64",
];

/// Largest download any entry may declare (a Node runtime archive is ~58 MB; a vendor repository archive can be
/// larger than the files we take from it).
pub const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// The whole catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    /// Format version; this build reads 1.
    pub schema: u32,
    /// The catalog's own version (shown in the UI, stored in receipts).
    pub version: String,
    /// The entries, in display order.
    pub entries: Vec<Entry>,
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
// `Entry` alone would collide with other generated names in the TypeScript index (its import finder matches words).
#[cfg_attr(feature = "ts", ts(rename = "CatalogEntry"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    /// Stable id; also the plugin folder name for packs and plugins.
    pub id: String,
    /// What kind it is.
    pub kind: Kind,
    /// Display name.
    pub name: String,
    /// Who makes it (the vendor, not us).
    pub publisher: String,
    /// Category id (see [`CATEGORIES`]).
    pub category: String,
    /// One plain line: what it is for.
    pub summary: String,
    /// SPDX licence expression of what gets installed (`n/a` for a remote server).
    pub licence: String,
    /// The vendor's page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub homepage: Option<String>,
    /// How it was reviewed (shown in the detail).
    pub review: String,
    /// Plain notes for the detail panel (what it sends where, what runs, limits).
    #[serde(default)]
    pub notes: Vec<String>,
    /// A Vibe connector alias or a catalog server id this works with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pairs_with: Option<Pairing>,
    /// Kind `pack`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pack: Option<Pack>,
    /// Kind `server`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub server: Option<Server>,
    /// Kind `plugin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub plugin: Option<PluginSpec>,
}

/// The kind of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A skill pack (installed as a Vibe plugin).
    Pack,
    /// A remote MCP server (added through the Connectors flow).
    Server,
    /// A local plugin that runs code (installed as a Vibe plugin with its runtime).
    Plugin,
}

/// What an entry works with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pairing {
    /// A connector alias in Vibe's catalog (`atlassian`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub connector: Option<String>,
    /// A `server` entry of this catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub server: Option<String>,
}

/// A skill pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pack {
    /// Shown version (for vendor files: the short commit).
    pub version: String,
    /// Where the vendor files come from (absent: a pack with only an `mcp.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<RepoSource>,
    /// The vendor files taken, each pinned.
    #[serde(default)]
    pub files: Vec<FileSpec>,
    /// The vendor's licence file, copied as `LICENSE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub licence_file: Option<FileSpec>,
    /// Literal edits applied after the hash check (research 11 generic edits), each result pinned too.
    #[serde(default)]
    pub edits: Vec<Edit>,
    /// Servers written into the pack's `mcp.json` (no sign-in, no headers).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, McpServerSpec>"))]
    pub mcp_servers: BTreeMap<String, McpServerSpec>,
}

/// A GitHub repository at one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoSource {
    /// `owner/name`.
    pub repo: String,
    /// The full 40-character commit.
    pub commit: String,
    /// Where the tar.gz archive is fetched (absent: GitHub's archive of `repo` at `commit`). Test indexes point it
    /// at a loopback server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub archive_url: Option<String>,
    /// Largest archive accepted, in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_bytes: u64,
}

impl RepoSource {
    /// The archive's address.
    #[must_use]
    pub fn url(&self) -> String {
        self.archive_url.clone().unwrap_or_else(|| {
            format!(
                "https://codeload.github.com/{}/tar.gz/{}",
                self.repo, self.commit
            )
        })
    }
}

/// One file taken from an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileSpec {
    /// Path inside the archive, below its single top folder.
    pub from: String,
    /// Path inside the installed plugin (packs: under `skills/`; the licence file's is ignored).
    pub to: String,
    /// SHA-256 of the bytes, lowercase hex.
    pub sha256: String,
}

/// A literal edit of one installed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Edit {
    /// The file (`to` of a [`FileSpec`]).
    pub file: String,
    /// Text that must occur exactly once.
    pub find: String,
    /// Its replacement.
    pub replace: String,
    /// SHA-256 of the file after all of its edits.
    pub result_sha256: String,
}

/// A server in a plugin's `mcp.json` (Agent Plugins 1.0 shapes Vibe accepts, `_native.py` `_MCPStdioServer` /
/// `_MCPStreamableHTTPServer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpServerSpec {
    /// A remote server that needs no sign-in.
    StreamableHttp {
        /// `https://...` (or loopback `http://`).
        url: String,
    },
    /// A local process.
    Stdio {
        /// `${RUNTIME}` (the plugin's runtime) or `./...` inside the plugin.
        command: String,
        /// Arguments; `${DATA}` is expanded by us, `${PLUGIN_ROOT}`/`${PLUGIN_DATA}` by Vibe.
        #[serde(default)]
        args: Vec<String>,
        /// Environment; same expansion.
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(type = "Record<string, string>"))]
        env: BTreeMap<String, String>,
    },
}

/// A remote MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Server {
    /// The server's address.
    pub url: String,
    /// Who can use it.
    pub access: Access,
    /// What was measured and when (`2026-10-10: asks for sign-in; offers app sign-up (DCR)`).
    pub checked: String,
    /// The vendor's own rule, quoted (approved-only and own-app servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rule: Option<String>,
    /// Where the rule is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub rule_source: Option<String>,
    /// What the user can do instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub what_to_do: Option<String>,
}

/// Who can use a remote server from Vibe Work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Access {
    /// OAuth with dynamic client registration: Vibe signs in.
    SignIn,
    /// The vendor allows approved apps only.
    ApprovedOnly,
    /// Needs the user's own app registration (no dynamic registration).
    OwnApp,
    /// Needs an API key: added with the user's key from the vault.
    Key,
}

/// A local plugin that runs code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSpec {
    /// Shown version (the main package's).
    pub version: String,
    /// The runtime, per platform.
    pub runtime: Runtime,
    /// npm packages, unpacked into `node_modules/<name>/` from their verified tarballs (no npm, no scripts).
    #[serde(default)]
    pub npm: Vec<NpmPackage>,
    /// One step run with the runtime after the files are in place (e.g. a browser download).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub post_install: Option<PostInstall>,
    /// The plugin's `mcp.json` servers (templates).
    #[cfg_attr(feature = "ts", ts(type = "Record<string, McpServerSpec>"))]
    pub mcp_servers: BTreeMap<String, McpServerSpec>,
    /// Our own files written into the plugin (`to` -> bundled file name, see `assemble::bundled`).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, string>"))]
    pub bundled: BTreeMap<String, String>,
    /// Plain sentence for the consent dialog: what runs and what it downloads.
    pub consent: String,
    /// Approximate download size in MB (shown before installing).
    pub download_mb: u32,
    /// Approximate disk use in MB.
    pub disk_mb: u32,
}

/// A runtime pinned per platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    /// `node`.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Per platform key (`linux-x64`, ...).
    #[cfg_attr(feature = "ts", ts(type = "Record<string, RuntimeTarget>"))]
    pub targets: BTreeMap<String, RuntimeTarget>,
}

/// The runtime for one platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTarget {
    /// Archive address.
    pub url: String,
    /// SHA-256 of the archive (the publisher's `SHASUMS256.txt`).
    pub sha256: String,
    /// `tarGz` or `zip`.
    pub format: ArchiveFormat,
    /// Archive size in bytes (the download is refused beyond it).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// Members taken (paths with the archive's top folder).
    pub files: Vec<Member>,
    /// The runtime executable inside the plugin (`runtime/node`, `runtime/node.exe`).
    pub executable: String,
}

/// One archive member taken as is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Member {
    /// Path inside the archive.
    pub from: String,
    /// Path inside the plugin.
    pub to: String,
    /// Mark executable (Unix).
    #[serde(default)]
    pub executable: bool,
}

/// An archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ArchiveFormat {
    /// gzip-compressed tar.
    TarGz,
    /// zip (deflate).
    Zip,
}

/// One npm package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NpmPackage {
    /// Package name (`@playwright/mcp`).
    pub name: String,
    /// Exact version.
    pub version: String,
    /// Tarball address.
    pub url: String,
    /// The registry's SRI `sha512-<base64>`.
    pub integrity: String,
    /// Largest tarball accepted, in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub max_bytes: u64,
}

/// A post-install step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostInstall {
    /// Arguments after the runtime executable; `${ROOT}` = the plugin folder being built, `${DATA}` = the entry's
    /// data folder.
    pub args: Vec<String>,
    /// Extra environment, same expansion.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, string>"))]
    pub env: BTreeMap<String, String>,
    /// Plain words for the progress line.
    pub what: String,
    /// Longest it may run.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub timeout_secs: u64,
}

/// Category ids in display order with their headings.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("code", "Code hosting & CI"),
    ("docs", "Docs & knowledge"),
    ("design", "Design"),
    ("data", "Data & databases"),
    ("observability", "Monitoring & errors"),
    ("search", "Search & research"),
    ("files", "Files & storage"),
    ("browser", "Browser automation"),
    ("work", "Work & projects"),
];

/// Options for [`Catalog::validate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Rules {
    /// Accept `http://127.0.0.1`/`localhost` addresses (test indexes only).
    pub allow_loopback_http: bool,
    /// Require every `plugin` entry to cover [`REQUIRED_TARGETS`] (the built-in catalog).
    pub require_all_targets: bool,
}

impl Catalog {
    /// Parses and validates `text`.
    ///
    /// # Errors
    /// [`MarketError::Catalog`] naming the entry and the rule.
    pub fn parse(text: &str, rules: Rules) -> Result<Self, MarketError> {
        let catalog: Self = serde_json::from_str(text)
            .map_err(|e| MarketError::Catalog(format!("not readable: {e}")))?;
        catalog.validate(rules)?;
        Ok(catalog)
    }

    /// The built-in catalog.
    ///
    /// # Errors
    /// [`MarketError::Catalog`] (a build with a broken catalog; the gate's `vw_market_catalog` test prevents it).
    pub fn built_in() -> Result<Self, MarketError> {
        Self::parse(
            BUILT_IN,
            Rules {
                allow_loopback_http: false,
                require_all_targets: true,
            },
        )
    }

    /// The entry with `id`.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Checks every rule in the module docs.
    ///
    /// # Errors
    /// [`MarketError::Catalog`] for the first broken rule.
    pub fn validate(&self, rules: Rules) -> Result<(), MarketError> {
        if self.schema != 1 {
            return Err(MarketError::Catalog(format!(
                "schema {} is not supported",
                self.schema
            )));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.entries {
            let fail = |rule: &str| MarketError::Catalog(format!("entry {:?}: {rule}", entry.id));
            if !is_plugin_name(&entry.id) || !ids.insert(entry.id.as_str()) {
                return Err(fail(
                    "id must be unique, 1-48 of a-z, 0-9 and single dashes",
                ));
            }
            if !CATEGORIES.iter().any(|(id, _)| *id == entry.category) {
                return Err(fail("unknown category"));
            }
            for text in [&entry.name, &entry.publisher, &entry.summary, &entry.review] {
                if text.trim().is_empty() || text.chars().any(char::is_control) {
                    return Err(fail("names and texts must be non-empty plain text"));
                }
            }
            if !is_licence(&entry.licence) {
                return Err(fail("licence must be an SPDX expression or n/a"));
            }
            if let Some(home) = &entry.homepage {
                check_url(home, false).map_err(|_| fail("homepage must be https"))?;
            }
            crate::catalog_rules::check_kind(entry, rules).map_err(|rule| fail(&rule))?;
        }
        Ok(())
    }
}

/// True for a usable plugin folder / Vibe plugin name (`_native.py` `_PluginManifest.name`, kept stricter: no dots).
#[must_use]
pub fn is_plugin_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 48
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.first() != Some(&b'-')
        && bytes.last() != Some(&b'-')
        && !name.contains("--")
}

fn is_licence(text: &str) -> bool {
    text == "n/a"
        || (!text.is_empty()
            && text.split(' ').all(|part| {
                matches!(part, "AND" | "OR")
                    || (!part.is_empty()
                        && part
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
            }))
}

/// Checks an address: `https://` with a host, or (when allowed) `http://` to a loopback host.
///
/// # Errors
/// A short reason (never the address).
pub fn check_url(url: &str, allow_loopback_http: bool) -> Result<(), &'static str> {
    if url.len() > 2048 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("address too long or contains spaces");
    }
    let rest = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        if !(allow_loopback_http && matches!(host, "127.0.0.1" | "localhost")) {
            return Err("only https addresses are accepted");
        }
        rest
    } else {
        return Err("only https addresses are accepted");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.contains('@') || host.contains('\\') {
        return Err("the address has no usable host");
    }
    Ok(())
}

/// The platform key of this build (`linux-x64`, `darwin-arm64`, `win-x64`,...), `None` on others.
#[must_use]
pub fn current_target() -> Option<&'static str> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return None;
    };
    Some(match (os, arch) {
        ("linux", "x64") => "linux-x64",
        ("linux", _) => "linux-arm64",
        ("darwin", "x64") => "darwin-x64",
        ("darwin", _) => "darwin-arm64",
        ("win", "x64") => "win-x64",
        _ => "win-arm64",
    })
}
