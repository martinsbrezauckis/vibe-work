//! The connectors part of the command API: the requests both clients send inside
//! `Command::Connectors`, what comes back, and the connectors and MCP servers as the UI sees them.
//!
//! Depends on: serde. Called by: `command`, `event`, `connectors` (the handler), the UI through the generated
//! TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - **No secret is ever part of a view** (a key server shows its secret's name and header only). Sign-in tokens stay at Mistral (hosted connectors) or in Vibe's
//!   keyring (custom servers); a server's address is shown without query or fragment, and no header,
//!   environment variable or command line from Vibe's config is copied here.
//! - Vibe's own words are kept where they explain a state (`diagnostic`, `error`): the user sees Vibe's
//!   reason, not a guess.

use serde::{Deserialize, Serialize};

use crate::keys::SecretInput;

/// One request about connectors and MCP servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ConnectorsRequest {
    /// The catalog (loaded once if Vibe has not loaded it yet), the servers in Vibe Work's own config, and
    /// the live state of `sessionId` when that session has an open channel (never reopened for a read).
    Read {
        /// The session whose live state is merged in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Asks Mistral for the account's catalog again (after a sign-in in the browser), lets every open
    /// session pick up the result, then reads.
    Refresh {
        /// As in `read`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Starts the sign-in of a Mistral-hosted connector (`connector_catalog/auth/request`). Vibe needs a
    /// session for it; the session is reopened when it has no channel. The sign-in page's address arrives
    /// as the `connector_catalog/authUrl` notice.
    SignIn {
        /// The session to ask through.
        session_id: String,
        /// The connector.
        alias: String,
    },
    /// Turns a Mistral-hosted connector off (or on again) in Vibe's config, for every session.
    SetConnectorOff {
        /// The connector.
        alias: String,
        /// True: off.
        off: bool,
    },
    /// Adds a remote MCP server that signs in with OAuth (`mcp_catalog/add`). Always asks the native
    /// dialog (a new remote endpoint).
    AddServer {
        /// `https://...` (or `http://` on this computer); no query or fragment.
        url: String,
        /// Name in Vibe's config; Vibe derives one from the address when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        name: Option<String>,
    },
    /// Starts the sign-in of an added server (`mcp_catalog/login`) in the background: the answer is
    /// immediate, the page address arrives as the `mcp_catalog/authUrl` notice, the outcome as the
    /// `connectorSignIn` event. One at a time (Vibe's callback port is fixed).
    SignInServer {
        /// The server's name.
        name: String,
    },
    /// Stops waiting for the running server sign-in (Vibe itself keeps its callback open until its own
    /// timeout of 10 minutes; nothing can stop that from outside).
    CancelSignIn {},
    /// Signs out of a server: Vibe deletes its token from the keyring (`mcp_catalog/logout`).
    SignOutServer {
        /// The server's name.
        name: String,
    },
    /// Removes a server from Vibe's config and deletes its sign-in (`mcp_catalog/remove`).
    RemoveServer {
        /// The server's name.
        name: String,
    },
    /// Turns a server off (or on again) for every session (`mcp_catalog/toggle`).
    SetServerOff {
        /// The server's name.
        name: String,
        /// True: off.
        off: bool,
    },
    /// Adds a remote MCP server that needs an API key: the key stays in the vault and Vibe Work's MCP
    /// forwarder adds it to each request; Vibe's config gets only the forwarder's address. Asks the native dialog
    /// (allow until Vibe Work closes / always / don't) for every origin: the Debug API may start it, the user's
    /// answer in the window decides.
    AddKeyServer {
        /// `https://...` (or `http://` on this computer); no query or fragment.
        url: String,
        /// Name in Vibe's config; derived from the address when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        name: Option<String>,
        /// The vault secret to use (a project secret's id); or `newSecret`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        secret_id: Option<String>,
        /// A new secret to store and use (one-way: the answer has no value).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        new_secret: Option<NewServerSecret>,
        /// The header that carries the key as is; absent: `Authorization: Bearer <key>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        header: Option<String>,
    },
    /// Allows a key server's key again (after a restart or a revoke): the same native dialog.
    AllowServerKey {
        /// The server's name.
        name: String,
    },
}

/// A new secret for `addKeyServer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewServerSecret {
    /// Its name in the vault.
    pub name: String,
    /// The key or token.
    pub value: SecretInput,
}

/// What a connectors request answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConnectorsResult {
    /// `read`, `refresh` and every change: the state after it.
    View {
        /// Connectors and servers.
        view: ConnectorsView,
    },
    /// `signIn`: Vibe accepted the request; the page address follows as a notice.
    SignInStarted {
        /// The connector.
        alias: String,
        /// The session that asked.
        session_id: String,
        /// Vibe's request id (the notice carries the same id).
        request_id: String,
    },
    /// `signInServer`: the sign-in runs; see the request's docs.
    ServerSignInStarted {
        /// The server.
        name: String,
    },
    /// `addServer`.
    ServerAdded {
        /// The name Vibe saved the server under.
        name: String,
        /// False when the same address was already configured (Vibe kept the existing entry).
        created: bool,
        /// The state after it.
        view: ConnectorsView,
    },
}

/// How fresh the catalog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CatalogLoad {
    /// Loaded in this run of Vibe (`memory`).
    Fresh,
    /// From Vibe's cache file, at most ten minutes old (`fresh_cache`).
    Cached,
    /// Not loaded: no key, or Mistral could not be reached (`catalogError` says why when known).
    NotLoaded,
}

/// The connectors and servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsView {
    /// How fresh the catalog is.
    pub catalog: CatalogLoad,
    /// Why the catalog could not be loaded (Vibe's message), when it could not.
    pub catalog_error: Option<String>,
    /// The current key can use shared connectors only; private sign-ins are unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub private_connectors_hidden: Option<bool>,
    /// Mistral-hosted connectors, in Vibe's order.
    pub connectors: Vec<ConnectorView>,
    /// MCP servers: from Vibe Work's own config and, when a session's state was read, from that session
    /// (plugins, servers of trusted project folders).
    pub servers: Vec<ServerView>,
    /// Mistral's console page for the account's connectors (where a connector's account link is
    /// removed); `null` when Vibe could not resolve the organization.
    pub manage_url: Option<String>,
    /// The session whose live state is merged in (`null`: none was open).
    pub session_id: Option<String>,
    /// The server whose sign-in is running now, if any.
    pub signing_in: Option<String>,
}

/// One Mistral-hosted connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConnectorView {
    /// Vibe's id (`linear`, `github_app`, ...).
    pub alias: String,
    /// Vibe's display name (sometimes just the id).
    pub display_name: String,
    /// `ready`, `needs_auth`, `needs_setup` or `unavailable` (Vibe's words).
    pub readiness: String,
    /// `oauth`, `credentials_setup`, `none` or `unknown`.
    pub auth_action: String,
    /// Turned off in Vibe's config.
    pub off: bool,
    /// The connector's tools (empty until signed in).
    pub tools: Vec<ToolView>,
    /// Vibe's reason when it is unavailable.
    pub diagnostic: Option<String>,
    /// The status in the merged session (`connected`, `needs_auth`, `disabled`, ...), when read.
    pub session_status: Option<String>,
}

/// One tool of a connector or server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    /// The tool's name.
    pub name: String,
    /// Its description, when given.
    pub description: Option<String>,
    /// False when turned off (only known from a session's state).
    pub enabled: Option<bool>,
}

/// One MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    /// The name in Vibe's config (the stable id).
    pub name: String,
    /// What to show (Vibe's display name, else the name).
    pub display_name: String,
    /// `streamable-http`, `http`, `stdio` or what Vibe reports.
    pub transport: String,
    /// Scheme, host and path of a remote server's address (no query, no fragment); `null` for a server
    /// that runs on this computer.
    pub address: Option<String>,
    /// `oauth`, `static` (headers or a key in Vibe's config), `none` or `unknown`.
    pub sign_in: String,
    /// Turned off in Vibe's config.
    pub off: bool,
    /// The live status (`connected`, `needs_auth`, `disabled`, `unavailable`, ...) from the merged
    /// session, `null` when no session state was read.
    pub status: Option<String>,
    /// Tools, from the merged session.
    pub tools: Vec<ToolView>,
    /// Vibe's error text, when it has one.
    pub error: Option<String>,
    /// The plugin that provides it (`null`: Vibe's config).
    pub plugin: Option<String>,
    /// Origin: built-in for app-injected tools or a built-in plugin; otherwise the pinned plugin’s scope, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub source: Option<String>,
    /// True when it is in Vibe Work's own config (it can be signed out, turned off and removed here).
    pub configured: bool,
    /// The key Vibe Work adds for it (`null`: not a key server).
    pub key: Option<ServerKeyView>,
}

/// How a key server gets its key. Names only, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerKeyView {
    /// The vault secret's name (`null` while the vault is locked).
    pub secret_name: Option<String>,
    /// `Authorization: Bearer` or the custom header's name (`null` while locked).
    pub header: Option<String>,
    /// `always`, `untilClose` (allowed until Vibe Work closes), `none` (not allowed now), `locked` (vault locked).
    pub grant: String,
}
