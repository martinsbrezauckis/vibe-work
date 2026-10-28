//! The `secrets` command's requests and results:
//! Settings > Project secrets lists metadata and grants, adds, renames, replaces, generates, removes and copies secrets
//! and website logins. The user can pause agent vault access here. Mutations keep item ids where applicable; copies
//! go only to the native clipboard. `serverGrants` adds MCP server grants, revoked with the same `revoke`.
//! Logic lives in `secrets.rs` (commands) and `secret_uses.rs` (the in-memory use log); the agent
//! side in `agent_tools/secret_run.rs`.
//!
//! Depends on: `keys` (`SecretInput`), serde. Called by: `command` (the `secrets` variant), `secrets`,
//! `secret_uses`, the UI through the generated TypeScript.
//! Invariants: wire form `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused; no result carries a
//! secret value (names, ids, variable names, programs and times only); secret and password inputs are redacted in
//! `Debug` and when serialized. Vault access rules classify all seven actions as user-only.

use serde::{Deserialize, Serialize};

use crate::keys::SecretInput;

/// One `secrets` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SecretsRequest {
    /// The secrets an agent may name, the "always" grants and the latest uses.
    List {},
    /// Lists metadata with the permissions for one project folder.
    ListForProject {
        /// Project folder.
        project: String,
    },
    /// Sets one entry's agent permission in this project, without changing the entry itself.
    SetEntryPermission {
        /// Project folder.
        project: String,
        /// Secret or login id.
        id: String,
        /// New choice.
        permission: EntryPermissionView,
    },
    /// Sets this secret or login's entry-wide default, independent of project selection.
    SetDefaultPermission {
        /// Secret or login id.
        id: String,
        /// New default choice.
        permission: EntryPermissionView,
    },
    /// Removes a project's own choice so this entry inherits its default again.
    ResetEntryPermission {
        /// Project folder.
        project: String,
        /// Secret or login id.
        id: String,
    },
    /// Generates and stores a secret, then copies its value without returning it.
    Generate {
        /// Display name.
        name: String,
        /// Optional suggestion label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
    },
    /// Creates and copies a generated secret with a project permission in the same user command.
    GenerateWithPermission {
        /// Display name.
        name: String,
        /// Optional suggestion label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
        /// Project folder.
        project: String,
        /// Initial agent permission.
        permission: EntryPermissionView,
    },
    /// Generates a replacement value for an own secret, preserving its id and grants.
    Regenerate {
        /// Existing secret id.
        id: String,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
    },
    /// Generates and stores a website login, then copies its password.
    GenerateLogin {
        /// Display name.
        name: String,
        /// Website URL.
        url: String,
        /// User name.
        username: String,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
    },
    /// Creates and copies a generated login with a project permission in the same user command.
    GenerateLoginWithPermission {
        /// Display name.
        name: String,
        /// Website URL.
        url: String,
        /// User name.
        username: String,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
        /// Project folder.
        project: String,
        /// Initial agent permission.
        permission: EntryPermissionView,
    },
    /// Generates a replacement login password, preserving its id and grants.
    RegenerateLoginPassword {
        /// Existing login id.
        id: String,
        /// Password generation settings.
        #[serde(default)]
        options: PasswordOptions,
    },
    /// Changes the user-controlled agent access switch.
    SetAgentAccess {
        /// True to refuse agent vault access.
        locked: bool,
    },
    /// Stores `value` as `name` (an own secret of that name is replaced). One-way: the answer has no value.
    Add {
        /// The name people and agents use.
        name: String,
        /// The value (a key or token).
        value: SecretInput,
        /// `"mcp"` marks a key for an MCP server (a suggestion only, no effect on access); absent or
        /// any other word: a plain key. A replaced own secret keeps its label when none is given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
    },
    /// Stores a secret and applies its project permission in one user command.
    AddWithPermission {
        /// Display name.
        name: String,
        /// Secret value.
        value: SecretInput,
        /// Optional suggestion label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        label: Option<String>,
        /// Project folder.
        project: String,
        /// Initial agent permission.
        permission: EntryPermissionView,
    },
    /// Removes a secret Vibe Work created, and every grant of it.
    Remove {
        /// The secret's id.
        id: String,
    },
    /// Renames an own secret while keeping its id, fields, label, source and grants.
    Rename {
        /// Existing secret id.
        id: String,
        /// New display name.
        name: String,
    },
    /// Replaces only the value of an own secret.
    Replace {
        /// Existing secret id.
        id: String,
        /// New secret value.
        value: SecretInput,
    },
    /// Copies a listed secret's value to the short-lived native clipboard.
    Copy {
        /// Listed secret id.
        id: String,
    },
    /// Revokes one "always" grant (an MCP server's key or a website fill grant).
    Revoke {
        /// The grant's id.
        grant_id: String,
    },
    /// Revokes several grants in one request ("Revoke all" on the Allowed without asking page): MCP server key and website fill grants alike. Ids already gone are skipped; none left is `notFound`.
    RevokeMany {
        /// The grants' ids (at most 10 000).
        grant_ids: Vec<String>,
    },
    /// stores a website login (Vault's plain `login` item) the agent may fill in after the user's consent.
    /// One-way: the answer has no password.
    AddLogin {
        /// The name people and agents use (empty: the site's host).
        #[serde(default)]
        name: String,
        /// The site (`https://example.com`, or a host name).
        url: String,
        /// The user name (the agent may see it).
        #[serde(default)]
        username: String,
        /// The password (8 to 4096 bytes), kept exactly as typed (spaces included).
        password: LoginPassword,
    },
    /// Stores a login and applies its project permission in one user command.
    AddLoginWithPermission {
        /// Display name.
        name: String,
        /// Website URL.
        url: String,
        /// User name.
        username: String,
        /// Password value.
        password: LoginPassword,
        /// Project folder.
        project: String,
        /// Initial agent permission.
        permission: EntryPermissionView,
    },
    /// removes a login Vibe Work created, and its fill grants.
    RemoveLogin {
        /// The login's id.
        id: String,
    },
    /// Renames an own login without changing its site, user name or password.
    RenameLogin {
        /// Existing login id.
        id: String,
        /// New display name.
        name: String,
    },
    /// Replaces only an own login's password, preserving its exact bytes.
    ReplaceLoginPassword {
        /// Existing login id.
        id: String,
        /// New password, preserved exactly.
        password: LoginPassword,
    },
    /// Copies a listed login's user name to the short-lived native clipboard.
    CopyLoginUsername {
        /// Listed login id.
        id: String,
    },
    /// Copies a listed login's password to the short-lived native clipboard.
    CopyLoginPassword {
        /// Listed login id.
        id: String,
    },
}

/// Settings for the shared vault password generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PasswordOptions {
    /// Number of characters, from 8 through 128.
    pub length: usize,
    /// Include uppercase letters.
    pub uppercase: bool,
    /// Include digits.
    pub digits: bool,
    /// Include symbols.
    pub symbols: bool,
}

impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            length: 24,
            uppercase: true,
            digits: true,
            symbols: true,
        }
    }
}

/// A login's password on its way into the vault: kept exactly as typed (a [`SecretInput`] trims, which would
/// sign in with the wrong value), redacted in `Debug` and when serialized.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct LoginPassword(vw_vault::SecretString);

impl LoginPassword {
    /// Wraps a value exactly (tests and adapters).
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(vw_vault::SecretString::exact(value))
    }

    /// True when the password starts with `prefix` (the private vault test switch accepts only test-prefixed
    /// throwaway values; private test builds only).
    #[cfg(feature = "vault-e2e")]
    pub(crate) fn has_prefix(&self, prefix: &str) -> bool {
        self.0.expose().starts_with(prefix)
    }

    pub(crate) fn into_secret(self) -> vw_vault::SecretString {
        self.0
    }
}

impl std::fmt::Debug for LoginPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for LoginPassword {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("<redacted>")
    }
}

impl<'de> Deserialize<'de> for LoginPassword {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = zeroize::Zeroizing::new(String::deserialize(deserializer)?);
        Ok(Self(vw_vault::SecretString::exact(raw.as_str())))
    }
}

/// A website login in the vault (metadata only, never the password).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LoginView {
    /// Item id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The site's origin.
    pub origin: String,
    /// The user name.
    pub username: String,
    /// Created by Vibe Work (it may remove it); other logins belong to the user's other Vault apps.
    pub own: bool,
    /// The user-selected permission in the requested project.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub permission: Option<EntryPermissionView>,
    /// Entry-wide choice, editable with no project selected.
    pub default_permission: EntryPermissionView,
    /// Explicit choice for the requested project, if present.
    pub project_permission: Option<EntryPermissionView>,
    /// All explicit project exceptions for this entry.
    pub project_permissions: Vec<ProjectPermissionView>,
}

/// One explicit project choice shown in Vault metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectPermissionView {
    /// Canonical project folder.
    pub project: String,
    /// User-selected choice.
    pub permission: EntryPermissionView,
}

/// Project permission on one Vault entry. Login site and MCP server grants keep their own scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum EntryPermissionView {
    /// Use existing narrower consent and grants.
    #[default]
    AskEachTime,
    /// Standing permission in this project.
    Allowed,
    /// Refuse agent use in this project.
    Never,
}

impl From<vw_keys::EntryPermission> for EntryPermissionView {
    fn from(value: vw_keys::EntryPermission) -> Self {
        match value {
            vw_keys::EntryPermission::AskEachTime => Self::AskEachTime,
            vw_keys::EntryPermission::Allowed => Self::Allowed,
            vw_keys::EntryPermission::Never => Self::Never,
        }
    }
}

impl From<EntryPermissionView> for vw_keys::EntryPermission {
    fn from(value: EntryPermissionView) -> Self {
        match value {
            EntryPermissionView::AskEachTime => Self::AskEachTime,
            EntryPermissionView::Allowed => Self::Allowed,
            EntryPermissionView::Never => Self::Never,
        }
    }
}

impl From<vw_keys::Login> for LoginView {
    fn from(l: vw_keys::Login) -> Self {
        Self {
            id: l.id,
            name: l.name,
            origin: l.origin,
            username: l.username,
            own: l.own,
            permission: Some(EntryPermissionView::AskEachTime),
            default_permission: EntryPermissionView::AskEachTime,
            project_permission: None,
            project_permissions: Vec::new(),
        }
    }
}

/// an "always for this site" fill grant: the agent may fill this login's password on exactly `origin`
/// without asking again, until it is revoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FillGrantView {
    /// Grant id (what `revoke` takes).
    pub id: String,
    /// The login's id.
    pub login_id: String,
    /// The login's name when it was allowed.
    pub login_name: String,
    /// The exact site.
    pub origin: String,
    /// When the user chose "always", ms.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub granted_ms: i64,
    /// Fills since Vibe Work started.
    pub uses: u32,
    /// The last fill since Vibe Work started, ms; `None` before the first.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_used_ms: Option<i64>,
}

/// A secret an agent may name (metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProjectSecretView {
    /// Resource id (what the agent passes as `secret`).
    pub id: String,
    /// Its name.
    pub name: String,
    /// `developerCredential` or `secret`.
    pub kind: String,
    /// Secret field names (never values).
    pub fields: Vec<String>,
    /// Created by Vibe Work (it may remove it); other secrets belong to the user or another app.
    pub own: bool,
    /// The user-selected permission in the requested project.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub permission: Option<EntryPermissionView>,
    /// Entry-wide choice, editable with no project selected.
    pub default_permission: EntryPermissionView,
    /// Explicit choice for the requested project, if present.
    pub project_permission: Option<EntryPermissionView>,
    /// All explicit project exceptions for this entry.
    pub project_permissions: Vec<ProjectPermissionView>,
    /// The page origin a captured key came from.
    pub source_origin: Option<String>,
    /// `"mcp"` for a key meant for an MCP server, else `null`. A suggestion only.
    pub label: Option<String>,
    /// Last change, ms since the epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated_ms: i64,
}

impl From<vw_keys::ProjectSecret> for ProjectSecretView {
    fn from(s: vw_keys::ProjectSecret) -> Self {
        Self {
            id: s.id,
            name: s.name,
            kind: s.kind.to_owned(),
            fields: s.fields,
            own: s.own,
            permission: Some(EntryPermissionView::AskEachTime),
            default_permission: EntryPermissionView::AskEachTime,
            project_permission: None,
            project_permissions: Vec::new(),
            source_origin: s.source_origin,
            label: s.label.map(|l| l.as_str().to_owned()),
            updated_ms: s.updated_ms,
        }
    }
}

/// An MCP server that gets a vault secret as its key, with the user's grant. Revoking it (`revoke` with
/// this `id`) stops Vibe Work's forwarder from adding the key until the user allows it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerGrantView {
    /// The binding's id (what `revoke` takes).
    pub id: String,
    /// The server's name in Vibe's config.
    pub server: String,
    /// Where the key goes (scheme, host, port and path).
    pub address: String,
    /// `Authorization: Bearer` or the custom header's name.
    pub header: String,
    /// The secret's name.
    pub secret_name: String,
    /// The secret's id (removing the secret revokes this).
    pub secret_id: String,
    /// `always`, `untilClose` or `none` (revoked, or allowed only in an earlier run).
    pub grant: String,
    /// Requests forwarded with the key in this run.
    pub uses: u32,
    /// When the binding was made (the key was given to the server), ms.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_ms: i64,
    /// The last request forwarded with the key in this run, ms; `None` before the first.
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_used_ms: Option<i64>,
}

/// One request of an agent to use secrets, and what happened (the use log; metadata only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SecretUseView {
    /// When, ms.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at_ms: i64,
    /// The session's title (or id).
    pub session: String,
    /// The project folder.
    pub project: String,
    /// The program that ran (or would have run).
    pub program: String,
    /// `NAME <- secret name` per variable.
    pub bindings: Vec<String>,
    /// `entryAllowed` (project permission), `allowedOnce`, `allowedAlways`, `grant` (an existing narrow grant),
    /// `declined`, `refused`.
    pub decision: String,
}

/// One `secrets` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SecretsResult {
    /// `list`.
    List {
        /// False when the vault is locked (the secrets are then unknown and `secrets` is empty).
        unlocked: bool,
        /// True when the user has paused agent access to project vault data.
        agent_access_locked: bool,
        /// The secrets an agent may name, by name.
        secrets: Vec<ProjectSecretView>,
        /// The uses since Vibe Work started, newest first (at most 200 are kept; never stored).
        uses: Vec<SecretUseView>,
        /// MCP servers that get a secret as their key (empty while the vault is locked).
        server_grants: Vec<ServerGrantView>,
        /// The website logins in the vault (empty while it is locked).
        logins: Vec<LoginView>,
        /// The "always for this site" fill grants in force (empty while the vault is locked).
        fill_grants: Vec<FillGrantView>,
    },
    /// `addLogin`.
    LoginStored {
        /// The stored login.
        login: LoginView,
        /// An own login of the same site and user name was replaced.
        replaced: bool,
    },
    /// `add`.
    Stored {
        /// The stored secret.
        secret: ProjectSecretView,
        /// An own secret of the same name was replaced.
        replaced: bool,
    },
    /// `remove` / `revoke`.
    Done {},
}
