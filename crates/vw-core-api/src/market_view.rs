//! The marketplace part of the command API: the requests both clients send inside
//! `Command::Marketplace`, what comes back, and the catalog as the UI sees it.
//!
//! Depends on: serde, vw-market (the catalog entry type). Called by: `command`, `event`, `market` (the handler), the UI
//! through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - A view never carries a secret: entries are our reviewed catalog; Vibe's plugin and skill state is reduced to
//!   names, descriptions, installed folders, statuses and discovery issues.

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
        /// The new-chat folder selected through the native picker, when no chat is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cwd: Option<String>,
    },
    /// Moves an owned plugin outside Vibe discovery.
    TurnOffPlugin {
        /// The installed folder shown by the board.
        path: String,
        /// The open chat determines the opened project.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// The new-chat folder picked natively, when no chat is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cwd: Option<String>,
    },
    /// Restores an owned plugin from the off folder.
    TurnOnPlugin {
        /// The installed folder shown by the board.
        path: String,
        /// The open chat determines the opened project.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// The new-chat folder picked natively, when no chat is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cwd: Option<String>,
    },
    /// Deletes an owned plugin after native confirmation.
    RemovePlugin {
        /// The installed folder shown by the board.
        path: String,
        /// The open chat determines the opened project.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
        /// The new-chat folder picked natively, when no chat is open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cwd: Option<String>,
    },
    /// Installs (or updates) a pack or plugin; runs in the background (progress and the outcome arrive as
    /// `marketInstall` events). One at a time. From the window with `offer_id` the click confirms that
    /// shown offer and no native dialog opens; without it the native dialog asks as before.
    Install {
        /// The catalog entry.
        id: String,
        /// The id of the `offerInstall` offer the window showed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        offer_id: Option<String>,
    },
    /// What Install would put on this computer (name, publisher, licence, what runs), as a single-use offer.
    OfferInstall {
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
    /// `offerInstall`: what the click would install; the click sends `offerId` back.
    Offer {
        /// The single-use offer.
        offer: crate::install_offers::ToolsInstallOffer,
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
    /// Every resolved plugin without a marketplace receipt, plus app-managed off plugins.
    pub discovered: Vec<LocalPluginView>,
    /// Why the current discovery could not be read (pinned chat state can still be shown).
    pub discovery_error: Option<String>,
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
    /// The installed plugin folder (including the off folder), absent for uninstalled entries.
    pub folder: Option<String>,
    /// This catalog install is kept outside Vibe's discovery roots.
    pub off: bool,
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
    /// On; `null` when this session does not offer the skill yet (the skills of a plugin installed
    /// during a session are offered to new sessions, or after a plugin reload).
    pub enabled: Option<bool>,
    /// Vibe reports the skill as `locked` (set by configuration, not per session): it is on while
    /// the plugin is installed.
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

/// A plugin discovered by Vibe outside a marketplace install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LocalPluginView {
    /// Vibe's plugin name.
    pub name: String,
    /// Vibe's description.
    pub description: String,
    /// The version its manifest gives; `null` when it gives none.
    pub version: Option<String>,
    /// The author its manifest gives; `null` when it gives none.
    pub author: Option<String>,
    /// Its installed folder, including for restored chats with a separate pinned checkout.
    pub path: String,
    /// Vibe's scope (`project`, `global`, `builtin`).
    pub scope: String,
    /// Only direct folders in the app's Vibe home or opened project may be changed.
    pub managed: bool,
    /// Kept in the app's off folder, outside Vibe discovery.
    pub off: bool,
    /// Vibe selected this installed folder for new chats (false for shadows and retained pins).
    pub available: bool,
    /// Current discovered components (skills, MCP servers and other kinds).
    pub components: Vec<ComponentView>,
    /// MCP servers with the chat's known tool count (`null`: not connected in this chat).
    pub servers: Vec<PluginServerView>,
    /// The open chat's pinned state; `null` when unavailable.
    pub in_session: Option<SessionPluginView>,
    /// Discovery issues and reasons Vibe selected another folder.
    pub issues: Vec<String>,
}

/// One plugin MCP server and its live tool count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PluginServerView {
    /// The source id Vibe gives it.
    pub name: String,
    /// Known tools in the chat; absent when no live catalog was read.
    pub tools: Option<usize>,
}
