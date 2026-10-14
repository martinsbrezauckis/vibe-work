//! The vault rule for the Debug API: outside the vault, the Debug API acts as the user.
//! For vault operations, a Debug API request has the agent's rights and no more.
//! Permission changes (grant, always, revoke), vault management (create, unlock, lock, passphrase, "remember this
//! computer", setting, replacing or removing the Mistral key, adding or removing secrets with values) and raw values
//! are the user's, in the window, by hand. Reason: the vault will become the main ShellX Vault, which has no Debug
//! access at all.
//!
//! This module is the one place that decides it:
//! - [`ACTS`]: every vault act a client command can ask for, by `"<command>.<op>"`, with its [`VaultAccess`]. A vault
//!   act the table does not name counts as user-only (fail closed: every `keys` and `secrets` op is a vault act, and the
//!   `connectors` and `browser` requests are classified by exhaustive matches, so a new variant does not compile until
//!   it is classified). A new vault method joins with its table line (and, for those two commands, its match arm).
//! - [`DIALOG_METHODS`]: the confirmation methods only the native dialog answers, also when the Debug API asked
//!   (`confirmer.rs` reads it): the Debug API may start such a flow, but the user's answer in the
//!   window decides. A new vault dialog joins this list with one line.
//! - [`admit`]: called by `Core::execute` for every command before it is dispatched, with the command's origin. A
//!   window command run under a live Debug API UI-control capability arrives here as `Origin::DebugApi` too
//!   (`vw-host` `execute_for_ui_control`), so a Revoke clicked through `ui/click` is refused the same way.
//!
//! Depends on: `command` (the commands), `keys`, `secrets_view`, `connectors_view`, `browser_view` (their request
//! types), `mcp_keys` (bindings, dialog method), `connectors_config` (the configured servers), `browser_secret` and
//! `agent_tools::secret_run` (dialog methods). Called by: `dispatch` (`Core::execute`), `confirmer`.
//! Invariants:
//! - A user-only act from `Origin::DebugApi` is refused before anything runs: `denied`, reason word
//!   [`VAULT_USER_ONLY`], never a partial change.
//! - The window (`Origin::Ui`) is never restricted here; the agent never comes through here (agent tools ask the
//!   confirmer with requester `Agent`, which is never auto-approved).
//! - Reads stay readable: names, states, grant lists are metadata the window shows
//!   too; no route offers a value (the secret guard).
//! - Private test builds only (cargo feature `vault-e2e`, refused in release builds): with the test switch's three
//!   locks a user-only act from the Debug API runs as the window's on a throwaway test vault with test-prefixed
//!   values; the switch's code is not part of the public source.

use serde::Serialize;
use tracing::info;

use crate::browser_view::BrowserRequest;
use crate::command::{Command, Origin};
use crate::connectors_view::ConnectorsRequest;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};

/// Private test builds only: the vault test switch. The source lives outside the exported tree.
#[cfg(feature = "vault-e2e")]
// rustfmt must not follow it: the file is absent from the public export, where `cargo fmt` would fail on it.
#[rustfmt::skip]
#[path = "../../../private/vault-e2e/core.rs"]
pub mod vault_e2e;

/// What a Debug API request may do with one vault act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultAccess {
    /// Metadata only (states, names, grant lists): as before, no value is ever part of an answer.
    Read,
    /// Runs, but its native dialog (this confirmation method) decides and the Debug API never answers it; the method
    /// must be in [`DIALOG_METHODS`] (a private test keeps the two lists equal).
    Dialog(&'static str),
    /// The user's alone, in the window: refused to the Debug API.
    UserOnly,
    /// User-only when the named MCP server gets a vault key (removing it drops the key's grant); otherwise.
    UserOnlyForKeyServer,
}

/// Every vault act a client command can ask for. One line per act; [`vault_act`] names a command's act.
pub const ACTS: &[(&str, VaultAccess)] = &[
    // The Mistral key and the vault.
    ("keys.readStatus", VaultAccess::Read),
    ("keys.listKeyCandidates", VaultAccess::Read),
    ("keys.detectVibeKey", VaultAccess::Read),
    // A `GET /v1/models` with the key in use; answers ok or the failure, never the key (every agent turn uses it too).
    ("keys.testKey", VaultAccess::Read),
    ("keys.createVault", VaultAccess::UserOnly),
    ("keys.unlockVault", VaultAccess::UserOnly),
    ("keys.lockVault", VaultAccess::UserOnly),
    ("keys.setupStoreKey", VaultAccess::UserOnly),
    ("keys.removeKey", VaultAccess::UserOnly),
    ("keys.useExistingKey", VaultAccess::UserOnly),
    ("keys.importVibeKey", VaultAccess::UserOnly),
    ("keys.removeVibeKeyOriginal", VaultAccess::UserOnly),
    // Project secrets and their grants.
    ("secrets.list", VaultAccess::Read),
    ("secrets.listForProject", VaultAccess::Read),
    ("secrets.setEntryPermission", VaultAccess::UserOnly),
    ("secrets.setDefaultPermission", VaultAccess::UserOnly),
    ("secrets.resetEntryPermission", VaultAccess::UserOnly),
    ("secrets.generate", VaultAccess::UserOnly),
    ("secrets.generateWithPermission", VaultAccess::UserOnly),
    ("secrets.regenerate", VaultAccess::UserOnly),
    ("secrets.generateLogin", VaultAccess::UserOnly),
    ("secrets.generateLoginWithPermission", VaultAccess::UserOnly),
    ("secrets.regenerateLoginPassword", VaultAccess::UserOnly),
    ("secrets.setAgentAccess", VaultAccess::UserOnly),
    ("secrets.add", VaultAccess::UserOnly),
    ("secrets.addWithPermission", VaultAccess::UserOnly),
    ("secrets.remove", VaultAccess::UserOnly),
    ("secrets.rename", VaultAccess::UserOnly),
    ("secrets.replace", VaultAccess::UserOnly),
    ("secrets.copy", VaultAccess::UserOnly),
    // A command grant, an MCP server's key grant, or a website fill grant.
    ("secrets.revoke", VaultAccess::UserOnly),
    // Website logins: a login carries a password.
    ("secrets.addLogin", VaultAccess::UserOnly),
    ("secrets.addLoginWithPermission", VaultAccess::UserOnly),
    ("secrets.removeLogin", VaultAccess::UserOnly),
    ("secrets.renameLogin", VaultAccess::UserOnly),
    ("secrets.replaceLoginPassword", VaultAccess::UserOnly),
    ("secrets.copyLoginUsername", VaultAccess::UserOnly),
    ("secrets.copyLoginPassword", VaultAccess::UserOnly),
    // MCP servers with a vault key.
    (
        "connectors.addKeyServer",
        VaultAccess::Dialog(crate::mcp_keys::METHOD),
    ),
    ("connectors.addKeyServer.newSecret", VaultAccess::UserOnly),
    (
        "connectors.allowServerKey",
        VaultAccess::Dialog(crate::mcp_keys::METHOD),
    ),
    ("connectors.removeServer", VaultAccess::UserOnlyForKeyServer),
    // A vault login filled into a sign-in field (`browser act {action: "fill"}`); never "always" from a
    // client: the dialog's answer decides.
    (
        "browser.act.fill",
        VaultAccess::Dialog(crate::browser_fill::METHOD),
    ),
    // Only the window can choose and save page content in the vault.
    ("browser.captureSecret", VaultAccess::UserOnly),
    ("browser.chooseSecret", VaultAccess::UserOnly),
    ("browser.saveSecret", VaultAccess::UserOnly),
    ("browser.cancelSecret", VaultAccess::UserOnly),
];

/// Native-only dialogs that have no Debug-accessible dialog line. The browser save is user-only.
pub const AGENT_ONLY_DIALOG_METHODS: &[&str] = &[
    crate::agent_tools::SECRET_RUN_METHOD,
    crate::browser_secret::METHOD,
    crate::secret_replace::SECRET_METHOD,
    crate::secret_replace::LOGIN_METHOD,
];

/// Confirmation methods only the native dialog answers, whoever asked (the confirmer never answers them for the
/// Debug API): giving an MCP server a vault key, storing a captured key, filling a vault login into a page,
/// running a command with a secret. These are the methods of the [`VaultAccess::Dialog`] lines plus
/// native-only methods without a Debug-accessible dialog line ([`AGENT_ONLY_DIALOG_METHODS`]); the private test
/// `vw_dv1_vault_scope` fails when the two lists drift in either direction.
pub const DIALOG_METHODS: &[&str] = &[
    crate::mcp_keys::METHOD,
    crate::browser_secret::METHOD,
    crate::browser_fill::METHOD,
    crate::agent_tools::SECRET_RUN_METHOD,
    crate::secret_replace::SECRET_METHOD,
    crate::secret_replace::LOGIN_METHOD,
];

/// The reason word of a refused user-only vault act.
pub const VAULT_USER_ONLY: &str = "vaultUserOnly";

/// The refusal's text for a Debug API vault request.
pub const VAULT_USER_ONLY_MESSAGE: &str =
    "The vault is managed in the window by you; the Debug API has the agent's access here.";

/// The `"<command>.<op>"` name of the vault act `command` asks for, or `None` when it touches no vault.
///
/// The `connectors` and `browser` requests are matched without a catch-all: a variant added later does not compile
/// until it is classified here, and a vault act named here without a line in [`ACTS`] is user-only
/// ([`vault_access`]). A new vault method is refused to the Debug API until it is classified.
#[must_use]
pub fn vault_act(command: &Command) -> Option<String> {
    match command {
        Command::Keys { request } => Some(format!("keys.{}", op_name(request))),
        Command::Secrets { request } => Some(format!("secrets.{}", op_name(request))),
        Command::Connectors { request } => connectors_act(request).map(str::to_owned),
        Command::Browser { request } => browser_act(request).map(str::to_owned),
        _ => None,
    }
}

/// The vault act of a `connectors` request (exhaustive, see [`vault_act`]).
fn connectors_act(request: &ConnectorsRequest) -> Option<&'static str> {
    match request {
        ConnectorsRequest::AddKeyServer {
            new_secret: Some(_),
            ..
        } => Some("connectors.addKeyServer.newSecret"),
        ConnectorsRequest::AddKeyServer { .. } => Some("connectors.addKeyServer"),
        ConnectorsRequest::AllowServerKey { .. } => Some("connectors.allowServerKey"),
        // A key server's removal drops its vault binding; other servers are (`UserOnlyForKeyServer`).
        ConnectorsRequest::RemoveServer { .. } => Some("connectors.removeServer"),
        // No vault: Vibe's catalog, OAuth sign-ins (tokens in Vibe's own keyring), plain servers, on/off.
        ConnectorsRequest::Read { .. }
        | ConnectorsRequest::Refresh { .. }
        | ConnectorsRequest::SignIn { .. }
        | ConnectorsRequest::SetConnectorOff { .. }
        | ConnectorsRequest::AddServer { .. }
        | ConnectorsRequest::SignInServer { .. }
        | ConnectorsRequest::CancelSignIn {}
        | ConnectorsRequest::SignOutServer { .. }
        | ConnectorsRequest::SetServerOff { .. } => None,
    }
}

/// The vault act of a `browser` request (exhaustive, see [`vault_act`]).
fn browser_act(request: &BrowserRequest) -> Option<&'static str> {
    match request {
        BrowserRequest::CaptureSecret { .. } => Some("browser.captureSecret"),
        BrowserRequest::ChooseSecret { .. } => Some("browser.chooseSecret"),
        BrowserRequest::SaveSecret { .. } => Some("browser.saveSecret"),
        BrowserRequest::CancelSecret { .. } => Some("browser.cancelSecret"),
        // The `fill` action puts a vault login into the page (`browser.rs` routes it the same way).
        BrowserRequest::Act { args, .. }
            if args.get("action").and_then(serde_json::Value::as_str) == Some("fill") =>
        {
            Some("browser.act.fill")
        }
        // No vault: page reads (key-like text hidden), other actions, state, annotations, screenshots.
        BrowserRequest::Tabs { .. }
        | BrowserRequest::OpenTab { .. }
        | BrowserRequest::SwitchTab { .. }
        | BrowserRequest::CloseTab { .. }
        | BrowserRequest::ReorderTabs { .. }
        | BrowserRequest::ReviewHandoff { .. }
        | BrowserRequest::HandoffTab { .. }
        | BrowserRequest::TakeBackTab { .. }
        | BrowserRequest::EnginePool {}
        | BrowserRequest::SetEnginePool { .. }
        | BrowserRequest::Permissions {}
        | BrowserRequest::SetPermissions { .. }
        | BrowserRequest::Accounts {}
        | BrowserRequest::SignIn { .. }
        | BrowserRequest::SignOut { .. }
        | BrowserRequest::AccountsLayout { .. }
        | BrowserRequest::CloseSignIn {}
        | BrowserRequest::Read { .. }
        | BrowserRequest::Act { .. }
        | BrowserRequest::State { .. }
        | BrowserRequest::Stop { .. }
        | BrowserRequest::Resume { .. }
        | BrowserRequest::Annotate { .. }
        | BrowserRequest::Capture { .. }
        | BrowserRequest::CancelAnnotate { .. }
        | BrowserRequest::CheckAnnotations { .. } => None,
    }
}

/// What the table says for `command`: `None` when it touches no vault; a vault act the table does not name (a new
/// `keys` or `secrets` op, a `connectors` or `browser` act classified in [`vault_act`] without a line) is
/// [`VaultAccess::UserOnly`] (fail closed).
#[must_use]
pub fn vault_access(command: &Command) -> Option<VaultAccess> {
    let act = vault_act(command)?;
    let listed = ACTS
        .iter()
        .find(|(name, _)| *name == act)
        .map(|(_, access)| *access);
    Some(listed.unwrap_or(VaultAccess::UserOnly))
}

/// The wire name of a request's op (`"op"` tag). The request types serialize secret inputs as `"<redacted>"`, so no
/// value is touched; a request that cannot be serialized gets a name no table line has (user-only).
fn op_name<T: Serialize>(request: &T) -> String {
    serde_json::to_value(request)
        .ok()
        .and_then(|v| v.get("op").and_then(|op| op.as_str()).map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned())
}

/// Admits `command` from `origin`: the origin it runs with, or the refusal of a user-only vault act asked by the
/// Debug API (see the module docs).
///
/// # Errors
/// `denied` with the reason word [`VAULT_USER_ONLY`].
pub(crate) async fn admit(
    shared: &Shared,
    origin: Origin,
    command: &Command,
) -> Result<Origin, CoreError> {
    if origin != Origin::DebugApi {
        return Ok(origin);
    }
    if matches!(
        command,
        Command::Secrets {
            request: crate::secrets_view::SecretsRequest::List {}
                | crate::secrets_view::SecretsRequest::ListForProject { .. }
        }
    ) && crate::agent_access::locked(shared)
    {
        let mut error = CoreError::new(ErrorCode::Denied, crate::agent_access::locked_message());
        error.server_code = Some("agentAccessLocked".to_owned());
        return Err(error);
    }
    let user_only = match vault_access(command) {
        None | Some(VaultAccess::Read | VaultAccess::Dialog(_)) => false,
        Some(VaultAccess::UserOnly) => true,
        Some(VaultAccess::UserOnlyForKeyServer) => match command {
            Command::Connectors {
                request: ConnectorsRequest::RemoveServer { name },
            } => is_key_server(shared, name).await,
            // The table names this access for `removeServer` only; any other command with it is refused.
            _ => true,
        },
    };
    if !user_only {
        return Ok(origin);
    }
    #[cfg(feature = "vault-e2e")]
    if vault_e2e::stands_in(shared, command)? {
        return Ok(Origin::Ui);
    }
    info!(
        act = vault_act(command).as_deref().unwrap_or("?"),
        "vault: a user-only act from the Debug API refused"
    );
    let mut error = CoreError::new(ErrorCode::Denied, VAULT_USER_ONLY_MESSAGE);
    error.server_code = Some(VAULT_USER_ONLY.to_owned());
    Err(error)
}

/// True when MCP server `name` gets a vault key: its configured address is Vibe Work's MCP forwarder, or a vault
/// binding names it. A locked vault still finds a forwarder entry by its address.
async fn is_key_server(shared: &Shared, name: &str) -> bool {
    let home = &shared.instructions.own_home;
    let forwarded = crate::connectors_config::configured_servers(home)
        .iter()
        .any(|s| {
            s.name == name
                && s.address
                    .as_deref()
                    .is_some_and(|a| a.contains(vw_keyproxy::MCP_PREFIX))
        });
    if forwarded {
        return true;
    }
    crate::mcp_keys::bindings(shared)
        .await
        .is_ok_and(|list| list.iter().any(|b| b.server == name))
}
