//! The marketplace part of the command API: the requests both clients send inside
//! `Command::Marketplace`, what comes back, and the catalog as the UI sees it.
//!
//! Depends on: serde, vw-market (the catalog entry type). Called by: `command`, `event`, `market` (the handler), the UI
//! through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A view never carries a secret: entries are our reviewed catalog; Vibe's plugin and skill state is reduced to
//!   names, statuses and its own issue sentences.

use serde::{Deserialize, Serialize};
use vw_market::Entry;

/// One request about the marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MarketRequest {
    /// The catalog with what is installed, plus the state in `sessionId` when that session has an open channel
    /// (never reopened for a read).
    Read {
        /// The session whose plugin and skill state is merged in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Installs (or updates) a pack or plugin. Asks the native dialog from the window; runs in the background
    /// (progress and the outcome arrive as `marketInstall` events). One at a time.
    Install {
        /// The catalog entry.
        id: String,
    },
    /// Stops the running install (its staging folder is deleted; nothing installed changes).
    CancelInstall {},
    /// Removes an installed pack or plugin (and its data folder). Only what the marketplace installed.
    Remove {
        /// The catalog entry.
        id: String,
    },
}

/// What a marketplace request answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MarketResult {
    /// `read`, `remove`, `cancelInstall`: the state after it.
    View {
        /// The marketplace (boxed: much larger than the other answer).
        view: Box<MarketView>,
        /// A successful removal's data folder could not be deleted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        notice: Option<String>,
    },
    /// `install`: started; follow the `marketInstall` events.
    InstallStarted {
        /// The entry.
        id: String,
    },
}

/// The marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketView {
    /// The catalog's version (`null` when the catalog could not be read; `error` says why).
    pub catalog_version: Option<String>,
    /// Why the catalog is unavailable.
    pub error: Option<String>,
    /// Categories in display order (id, heading).
    pub categories: Vec<CategoryView>,
    /// The entries in catalog order.
    pub entries: Vec<MarketEntryView>,
    /// This computer's platform key (`linux-x64`, ...), `null` when the catalog has no runtimes for it.
    pub platform: Option<String>,
    /// The install running now.
    pub installing: Option<InstallProgress>,
    /// The session whose state is merged in (`null`: none was open).
    pub session_id: Option<String>,
    /// Plugin files Vibe could not load in that session (Vibe's sentences), ours or not.
    pub dropped: Vec<DroppedView>,
}

/// A category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CategoryView {
    /// Id.
    pub id: String,
    /// Heading.
    pub label: String,
}

/// One catalog entry with its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MarketEntryView {
    /// The catalog entry as reviewed.
    pub entry: Entry,
    /// What is installed, when it is.
    pub installed: Option<InstalledView>,
    /// A folder of this name exists that the marketplace did not install (left alone; install refused).
    pub foreign: bool,
    /// Packs and plugins: false when this computer's platform has no runtime for it.
    pub installable: bool,
    /// The merged session's view of the plugin (`null`: no session state was read).
    pub in_session: Option<SessionPluginView>,
}

/// An install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstalledView {
    /// The entry's version installed.
    pub version: String,
    /// The catalog version it came from.
    pub catalog_version: String,
    /// When (milliseconds since the Unix epoch).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub installed_at_ms: u64,
    /// The catalog now has a different version of it.
    pub update_available: bool,
    /// A file differs from what was installed (checked for packs; `null` for plugins with a runtime).
    pub changed: Option<bool>,
}

/// One plugin as the merged session runs it (`plugin_catalog/read`, `skills/installed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionPluginView {
    /// The session runs this plugin (Vibe pinned it at start or at the last reload).
    pub loaded: bool,
    /// Its components (`skill`, `tool`, `mcp`, ...) with Vibe's status when it reports one.
    pub components: Vec<ComponentView>,
    /// The plugin's skills (from the plugin's components) with their on/off state.
    pub skills: Vec<SkillStateView>,
    /// Vibe's sentences about this plugin in the session (an MCP server that could not be connected, ...).
    pub issues: Vec<String>,
}

/// A plugin component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ComponentView {
    /// Vibe's kind.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// Vibe's route status for a tool (`live`, ...), else `null`.
    pub status: Option<String>,
}

/// A skill's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SkillStateView {
    /// The skill's name (what `/name` runs).
    pub name: String,
    /// Its description.
    pub description: String,
    /// On; `null` when this session does not offer the skill yet (Vibe 2.26.0 offers the skills
    /// of a plugin installed during a session only to new sessions; 2.26.1 after a reload).
    pub enabled: Option<bool>,
    /// Vibe does not let it be switched off (Vibe 2.26 marks every plugin skill `locked`,
    /// `_unified_harness_backend_adapter.py:1303`): it is on while the plugin is installed.
    pub locked: bool,
}

/// A plugin file Vibe could not load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct DroppedView {
    /// The plugin it belongs to, when known (matched by Vibe's pinned folder).
    pub plugin: Option<String>,
    /// The plugin folder (last path parts only, never a full path).
    pub file: String,
    /// Vibe's sentence.
    pub message: String,
}

/// The running install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    /// The entry.
    pub id: String,
    /// `download`, `unpack`, `setup`, `finish`, then the outcome `installed`, `failed` or `cancelled`.
    pub phase: String,
    /// What (a package, an archive, the setup step's words).
    pub what: String,
    /// Bytes so far (downloads).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub done: Option<u64>,
    /// Bytes expected at most (downloads).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub total: Option<u64>,
    /// The error sentence when it failed.
    pub message: Option<String>,
}
