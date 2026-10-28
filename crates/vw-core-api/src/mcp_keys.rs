//! MCP servers that need an API key: the `addKeyServer` and `allowServerKey` connector requests, the
//! grants ("until Vibe Work closes" in memory, "always" in the binding's vault record), and the resolver the model
//! proxy's MCP forwarder asks on every request.
//!
//! How a key travels: vault secret -> this resolver (per request, only while a grant holds) -> the forwarder's
//! outgoing header -> the server the user confirmed. Vibe's config holds only the forwarder's address and the name of
//! the variable with the proxy token (`mcp_keys_config`); Vibe's process never holds the key, the same rule as for
//! the Mistral key.
//!
//! Depends on: vw-keys (secrets, bindings, resolver slot), vw-keyproxy (`McpResolver`), vw-gate (URL rule, dialog
//! values, confirmation types), `mcp_keys_config`, `connectors` (converge), `connectors_view`, `secrets_view`.
//! Called by: `connectors` (requests, view merge), `secrets` (list, revoke, secret removal), `core` (start, stop).
//! Invariants:
//! - No value is ever part of a result, an event, a dialog or a log line: names, ids, header names and addresses
//!   only. A value read for a request is registered as a stored secret (`vw_keys::secret_value`), so the Debug
//!   API's response guard and the agent tool scrub replace it wherever it might appear.
//! - The consent is always the native dialog (three answers), also when the Debug API started the request: the core's
//!   confirmer never answers this method for the Debug API (`vault_scope::DIALOG_METHODS`), and "always" comes only
//!   from the dialog's answer, never from a request. Declining writes nothing.
//! - A revoke takes effect on the next forwarded request (the resolver reads the grant every time).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use serde_json::json;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};
use vw_keyproxy::{McpFuture, McpRefusal, McpResolver, McpTarget};
use vw_keys::{KeyService, McpBinding};
use zeroize::Zeroizing;

use crate::command::Origin;
use crate::connectors_view::{NewServerSecret, ServerKeyView, ServerView};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::mcp_keys_config::{ConfigEditError, add_entry, forwarder_url};

/// The dialog's method name (not a Vibe method; the dialog shows it with the params).
pub const METHOD: &str = "vibe-work/mcpServerKey";
/// The dialog's "always" answer: its first button (the recommended choice; the default is the declining
/// "Don't allow", `vw_host::dialog_buttons`).
pub(crate) const ALWAYS_LABEL: &str = "Always (until revoked)";

/// Header names a key may not use: hop-by-hop, framing, and the ones MCP or the forwarder own.
const RESERVED_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "content-type",
    "accept",
    "cookie",
    "mcp-session-id",
    "mcp-protocol-version",
];

/// Plain grants and use counters. A plain "Allow" lasts until Vibe Work closes (memory only; the next start asks again);
/// "Always" is the vault record's own flag. An MCP connection needs the key on every
/// request, and Vibe's MCP config is shared by all sessions of one server, so a per-request or per-session scope is
/// not workable. No binding is cached: every decision reads the vault's current
/// record, so a revoked grant cannot remain usable through stale state.
#[derive(Default)]
pub(crate) struct McpKeysState {
    /// Binding id -> the record's `revokedMs` when the user gave the plain grant. The grant holds only while the
    /// record still carries that value: any revoke since (this window, another window or app on the same vault) moves
    /// it, so the decision never compares two wall-clock readings (a clock stepped back could otherwise let a plain
    /// grant outlive a revoke; 10 parallel test runs once failed at the same moment this way).
    plain: Mutex<HashMap<String, i64>>,
    /// Binding id -> (requests forwarded with its key in this run, the last one in ms).
    uses: Mutex<HashMap<String, (u32, i64)>>,
}

impl McpKeysState {
    fn plain(&self) -> std::sync::MutexGuard<'_, HashMap<String, i64>> {
        self.plain.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Times binding `id` was used in this run, and the last use (ms; `None` before the first).
    pub(crate) fn uses(&self, id: &str) -> (u32, Option<i64>) {
        self.uses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .map_or((0, None), |(n, last)| (*n, Some(*last)))
    }

    fn count(&self, id: &str) {
        let mut uses = self.uses.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = uses.entry(id.to_owned()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = crate::fill_grants::now_ms();
    }

    /// The grant word of `b` for views: `always`, the plain scope's word, or `none`.
    fn grant_word(&self, b: &McpBinding) -> &'static str {
        if b.always {
            "always"
        } else if self.plain_holds(b) {
            "untilClose"
        } else {
            "none"
        }
    }

    /// True when this run's plain grant of `b` exists and no revoke happened since it was given (the record's
    /// `revokedMs` is still the one seen then; a revoke in another window or app on the same vault changes it).
    fn plain_holds(&self, b: &McpBinding) -> bool {
        self.plain()
            .get(&b.id)
            .is_some_and(|seen| *seen == b.revoked_ms)
    }

    /// Decides one forwarded request for `b` (the vault's current record).
    fn permit(&self, b: &McpBinding) -> bool {
        b.always || self.plain_holds(b)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn keys(shared: &Shared) -> Result<&Arc<KeyService>, CoreError> {
    shared
        .config
        .keys
        .as_ref()
        .ok_or_else(crate::keys::no_vault)
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}

fn config_error(error: &ConfigEditError) -> CoreError {
    let code = match error {
        ConfigEditError::NameTaken => ErrorCode::InvalidParams,
        _ => ErrorCode::Internal,
    };
    CoreError::new(code, error.to_string())
}

/// The bindings, read from the vault now.
///
/// # Errors
/// The key service's error (a locked vault: `vaultLocked`).
pub(crate) async fn bindings(shared: &Shared) -> Result<Vec<McpBinding>, vw_keys::KeyError> {
    let Some(keys) = shared.config.keys.as_ref() else {
        return Ok(Vec::new());
    };
    keys.mcp_bindings().await
}

/// `header` checked: `None` = `Authorization` with `Bearer`; a custom name is a token (RFC 9110) of at most 64
/// characters and not reserved. Returns `(header, bearer)`.
fn checked_header(header: Option<&str>) -> Result<(String, bool), CoreError> {
    let Some(raw) = header.map(str::trim).filter(|h| !h.is_empty()) else {
        return Ok(("authorization".to_owned(), true));
    };
    let lower = raw.to_ascii_lowercase();
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    if raw.len() > 64 || !raw.bytes().all(token) {
        return Err(invalid(
            "the header name may have up to 64 letters, digits and - _ . characters",
        ));
    }
    if RESERVED_HEADERS.contains(&lower.as_str()) {
        return Err(invalid(
            "that header is used by the connection itself; pick another",
        ));
    }
    Ok((lower, false))
}

/// A Vibe server name: Vibe's own normalisation (`core/config/models.py:195-199`), from the request or the host.
fn server_name(requested: Option<&str>, url: &str) -> Result<String, CoreError> {
    let normalize = |text: &str| -> String {
        let mapped: String = text
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        mapped
            .trim_matches(|c| c == '_' || c == '-')
            .chars()
            .take(64)
            .collect()
    };
    if let Some(name) = requested.map(str::trim).filter(|n| !n.is_empty()) {
        let normalized = normalize(name);
        if normalized.is_empty() {
            return Err(invalid("the name must contain letters or digits"));
        }
        return Ok(normalized);
    }
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let skip = usize::from(labels.len() >= 3 && matches!(labels[0], "mcp" | "www" | "api"));
    let candidate = labels.get(skip).copied().unwrap_or("mcp");
    let normalized = normalize(candidate);
    Ok(if normalized.is_empty() {
        "mcp".to_owned()
    } else {
        normalized
    })
}

/// The header line shown to people (`Authorization: Bearer` or the custom name).
fn header_text(b: &McpBinding) -> String {
    if b.bearer {
        "Authorization: Bearer".to_owned()
    } else {
        b.header.clone()
    }
}

/// The consent: the native dialog for every origin (module docs). Returns true for "always".
async fn consent(
    shared: &Shared,
    origin: Origin,
    server: &str,
    address: &str,
    header: &str,
    secret_name: &str,
) -> Result<bool, CoreError> {
    let request = ConfirmationRequest {
        method: METHOD.to_owned(),
        requester: crate::command::requester(origin),
        summary: consent_summary(server, address, header, secret_name),
        params: json!({"server": server, "address": address, "header": header, "secret": secret_name}),
        always: Some(ALWAYS_LABEL.to_owned()),
    };
    match shared.confirmer.confirm(&request).await {
        Confirmation::Approved => Ok(false),
        Confirmation::ApprovedAlways => Ok(true),
        answer @ Confirmation::Declined => Err(CoreError::new(
            ErrorCode::ConfirmationDeclined,
            answer.agent_text("this use of a secret"),
        )),
    }
}

fn consent_summary(server: &str, address: &str, header: &str, secret_name: &str) -> String {
    format!(
        "Let Vibe Work send your secret {} to the MCP server {} at {} in the header {}. Vibe gets only an \
             address on this computer; Vibe Work adds the key to each request, so neither Vibe's config nor the agent's \
             commands hold it. The server gets the key and can use it like any app you give it to. \"{}\" \
             (recommended) lasts until you revoke it in Settings > Project secrets; \"Allow until Vibe Work closes\" \
             asks again at the next start.",
        vw_gate::dialog_quote(secret_name),
        vw_gate::dialog_quote(server),
        vw_gate::dialog_quote(address),
        vw_gate::dialog_quote(header),
        ALWAYS_LABEL
    )
}

/// The secret to use: an existing project secret (by id) or a new one, stored only after the consent.
enum Chosen {
    Existing { id: String, name: String },
    New(NewServerSecret),
}

impl Chosen {
    fn name(&self) -> &str {
        match self {
            Self::Existing { name, .. } => name,
            Self::New(new) => new.name.trim(),
        }
    }
}

async fn choose(
    keys: &KeyService,
    secret_id: Option<String>,
    new_secret: Option<NewServerSecret>,
) -> Result<Chosen, CoreError> {
    match (secret_id, new_secret) {
        (Some(id), None) => {
            let list = keys
                .project_secrets()
                .await
                .map_err(|e| crate::keys::key_error(&e))?;
            let found = list
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| invalid("no such secret in the vault"))?;
            Ok(Chosen::Existing {
                id: found.id,
                name: found.name,
            })
        }
        (None, Some(new)) => Ok(Chosen::New(new)),
        _ => Err(invalid("give either secretId or newSecret")),
    }
}

/// Request fields of `addKeyServer` (kept together so the handler stays one readable function).
pub(crate) struct AddKeyServer {
    pub(crate) url: String,
    pub(crate) name: Option<String>,
    pub(crate) secret_id: Option<String>,
    pub(crate) new_secret: Option<NewServerSecret>,
    pub(crate) header: Option<String>,
}

/// `addKeyServer`: checks, consent, then the secret (when new), the vault record, Vibe's config entry. Returns the
/// server's name.
///
/// # Errors
/// `invalidParams` (address, name, header, secret choice, a name already configured), `confirmationDeclined`,
/// `keyService` (no vault, locked, a secret name taken), `internal` (config not editable).
pub(crate) async fn add(
    shared: &Shared,
    origin: Origin,
    req: AddKeyServer,
) -> Result<String, CoreError> {
    let keys = keys(shared)?;
    let url = vw_gate::check_mcp_server_url(&req.url).map_err(|r| invalid(r.to_string()))?;
    let (header, bearer) = checked_header(req.header.as_deref())?;
    let name = server_name(req.name.as_deref(), &url)?;
    let home = &shared.instructions.own_home;
    if crate::connectors_config::configured_servers(home)
        .iter()
        .any(|s| s.name == name)
    {
        return Err(invalid(format!(
            "an MCP server named {} is already configured",
            vw_gate::dialog_value(&name, 64)
        )));
    }
    let chosen = choose(keys, req.secret_id, req.new_secret).await?;
    let shown_header = if bearer {
        "Authorization: Bearer".to_owned()
    } else {
        header.clone()
    };
    let always = consent(shared, origin, &name, &url, &shown_header, chosen.name()).await?;
    let answered = now_ms();
    let (secret_id, secret_name) = match chosen {
        Chosen::Existing { id, name } => (id, name),
        Chosen::New(new) => {
            let stored = keys
                .store_secret_labelled(
                    new.name,
                    new.value.into_secret(),
                    None,
                    Some(vw_keys::SecretLabel::Mcp),
                )
                .await
                .map_err(|e| crate::keys::key_error(&e))?;
            (stored.secret.id, stored.secret.name)
        }
    };
    let binding = McpBinding {
        id: vw_vault::mcp_binding::new_id(),
        server: name.clone(),
        upstream: url,
        header,
        bearer,
        secret_id,
        secret_name,
        always,
        revoked_ms: 0,
        created_ms: now_ms(),
    };
    keys.store_mcp_binding(binding.clone())
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    if let Err(error) = add_entry(home, &name, &forwarder_url(keys.proxy_base(), &binding.id)) {
        // Nothing half-made: the record goes again (the secret stays: the user may use it elsewhere).
        if let Err(undo) = keys.remove_mcp_binding(binding.id.clone()).await {
            tracing::warn!(%undo, "mcp key server: the record could not be removed after a failed config edit");
        }
        return Err(config_error(&error));
    }
    if !always {
        shared
            .connectors
            .mcp_keys
            .plain()
            .insert(binding.id.clone(), binding.revoked_ms);
    }
    tracing::info!(always, answered, "mcp key server added");
    shared.emit(CoreEvent::SecretsChanged {});
    Ok(name)
}

/// `allowServerKey`: the same consent for an existing key server.
///
/// # Errors
/// `notFound` (no key server of that name), `confirmationDeclined`, `keyService`.
pub(crate) async fn allow(shared: &Shared, origin: Origin, name: &str) -> Result<(), CoreError> {
    let keys = keys(shared)?;
    let list = bindings(shared)
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    let binding = list.into_iter().find(|b| b.server == name).ok_or_else(|| {
        CoreError::new(ErrorCode::NotFound, "no MCP server with a key of that name")
    })?;
    let current_name = keys
        .project_secrets()
        .await
        .map_err(|e| crate::keys::key_error(&e))?
        .into_iter()
        .find(|secret| secret.id == binding.secret_id)
        .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "the server's vault key is missing"))?
        .name;
    let asked = now_ms();
    // The record's revoke stamp before the dialog: a revoke while it is open changes it (`revoke` always moves it).
    let revoked_at_ask = binding.revoked_ms;
    let chosen_always = consent(
        shared,
        origin,
        &binding.server,
        &binding.upstream,
        &header_text(&binding),
        &current_name,
    )
    .await?;
    let answered = now_ms();
    // The record as it is now, changed atomically (`update_mcp_binding`): a revoke made while the dialog was open
    // (its `revokedMs` at or after the moment the dialog opened) wins over the answer, which is then refused: fail
    // closed, the user can allow again.
    let after = keys
        .update_mcp_binding(binding.id.clone(), move |b| {
            if b.revoked_ms != revoked_at_ask || b.revoked_ms >= asked || !chosen_always || b.always
            {
                return false;
            }
            b.always = true;
            true
        })
        .await
        .map_err(|e| crate::keys::key_error(&e))?
        .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "the server's key record is gone"))?;
    if after.revoked_ms != revoked_at_ask || after.revoked_ms >= asked {
        let mut error = CoreError::new(
            ErrorCode::Denied,
            "the key was revoked while the dialog was open; choose Allow key again",
        );
        error.server_code = Some("revokedMeanwhile".to_owned());
        return Err(error);
    }
    if !chosen_always {
        // Given at the answer with the record's current revoke stamp: any revoke after it voids it (`plain_holds`).
        tracing::debug!(answered, "mcp key plain grant");
        shared
            .connectors
            .mcp_keys
            .plain()
            .insert(binding.id.clone(), after.revoked_ms);
    }
    shared.emit(CoreEvent::SecretsChanged {});
    Ok(())
}

/// Revokes binding `id` (both kinds of grant): `always` off and `revokedMs` now, written atomically on the vault's
/// current record, so plain grants of every process sharing the vault end too. A request the forwarder already
/// admitted finishes (its key was handed over before the revoke); the next one is refused. False when there is no
/// such binding.
///
/// # Errors
/// `keyService` (locked vault).
pub(crate) async fn revoke(shared: &Shared, id: &str) -> Result<bool, CoreError> {
    let keys = keys(shared)?;
    let now = now_ms();
    shared.connectors.mcp_keys.plain().remove(id);
    let after = keys
        .update_mcp_binding(id.to_owned(), move |b| {
            b.always = false;
            // Always forward (even when the clock stepped back), so every plain grant given before sees a change.
            b.revoked_ms = (b.revoked_ms + 1).max(now);
            true
        })
        .await
        .map_err(|e| crate::keys::key_error(&e))?;
    Ok(after.is_some())
}

/// Revokes every binding of secret `secret_id` (the secret was removed). Returns how many. A locked vault revokes
/// nothing (logged): the forwarder then cannot read the removed secret anyway.
pub(crate) async fn revoke_secret(shared: &Shared, secret_id: &str) -> usize {
    let Ok(list) = bindings(shared).await else {
        tracing::warn!("mcp key servers: bindings not read while removing a secret");
        return 0;
    };
    let mut n = 0;
    for b in list.into_iter().filter(|b| b.secret_id == secret_id) {
        if matches!(revoke(shared, &b.id).await, Ok(true)) {
            n += 1;
        }
    }
    n
}

/// Removes the vault record of key server `name`, if it is one (after Vibe removed its config entry).
pub(crate) async fn forget_server(shared: &Shared, name: &str) {
    let Some(keys) = shared.config.keys.as_ref() else {
        return;
    };
    let Ok(list) = bindings(shared).await else {
        return;
    };
    for b in list.into_iter().filter(|b| b.server == name) {
        shared.connectors.mcp_keys.plain().remove(&b.id);
        if let Err(error) = keys.remove_mcp_binding(b.id).await {
            tracing::warn!(%error, "mcp key server: its record could not be removed");
        }
    }
    shared.emit(CoreEvent::SecretsChanged {});
}

/// Adds the key view to Vibe Work's key servers in `servers` (and shows the confirmed address instead of the
/// forwarder's). Reads the vault's current bindings; while it is locked, forwarder entries show `locked`.
pub(crate) async fn merge_views(shared: &Shared, servers: &mut [ServerView]) {
    let list = bindings(shared).await.ok();
    let names = if let Some(keys) = shared.config.keys.as_ref() {
        keys.project_secrets().await.ok().unwrap_or_default()
    } else {
        Vec::new()
    };
    for server in servers.iter_mut() {
        let Some(binding) = list
            .as_ref()
            .and_then(|l| l.iter().find(|b| b.server == server.name))
        else {
            // A forwarder entry (shown by the address) whose record cannot be read now.
            if server
                .address
                .as_deref()
                .is_some_and(|a| a.contains(vw_keyproxy::MCP_PREFIX))
            {
                server.key = Some(ServerKeyView {
                    secret_name: None,
                    header: None,
                    grant: if list.is_none() { "locked" } else { "none" }.to_owned(),
                });
                "key".clone_into(&mut server.sign_in);
            }
            continue;
        };
        server.address = crate::connectors_config::public_address(&binding.upstream);
        "key".clone_into(&mut server.sign_in);
        server.key = Some(ServerKeyView {
            secret_name: Some(
                names
                    .iter()
                    .find(|s| s.id == binding.secret_id)
                    .map_or_else(|| binding.secret_name.clone(), |s| s.name.clone()),
            ),
            header: Some(header_text(binding)),
            grant: shared.connectors.mcp_keys.grant_word(binding).to_owned(),
        });
    }
}

/// The MCP servers that use a secret as their key, for Settings > Project secrets (`secrets list`).
pub(crate) async fn grant_views(shared: &Shared) -> Vec<crate::secrets_view::ServerGrantView> {
    let Ok(list) = bindings(shared).await else {
        return Vec::new();
    };
    let names = if let Some(keys) = shared.config.keys.as_ref() {
        keys.project_secrets().await.ok().unwrap_or_default()
    } else {
        Vec::new()
    };
    list.iter()
        .map(|b| crate::secrets_view::ServerGrantView {
            id: b.id.clone(),
            server: b.server.clone(),
            address: crate::connectors_config::public_address(&b.upstream).unwrap_or_default(),
            header: header_text(b),
            secret_name: names
                .iter()
                .find(|s| s.id == b.secret_id)
                .map_or_else(|| b.secret_name.clone(), |s| s.name.clone()),
            secret_id: b.secret_id.clone(),
            grant: shared.connectors.mcp_keys.grant_word(b).to_owned(),
            uses: shared.connectors.mcp_keys.uses(&b.id).0,
            created_ms: b.created_ms,
            last_used_ms: shared.connectors.mcp_keys.uses(&b.id).1,
        })
        .collect()
}

/// The resolver the proxy's MCP forwarder asks (module docs). Holds the core weakly: the key service outlives it.
struct Resolver {
    shared: Weak<Shared>,
}

impl McpResolver for Resolver {
    fn resolve<'a>(&'a self, id: &'a str) -> McpFuture<'a> {
        Box::pin(async move {
            let shared = self.shared.upgrade().ok_or(McpRefusal::Unknown)?;
            if crate::agent_access::locked(&shared) {
                return Err(McpRefusal::AgentAccessLocked);
            }
            let Some(keys) = shared.config.keys.as_ref() else {
                return Err(McpRefusal::Unknown);
            };
            // The vault's current record, read for this request (never a copy): a record revoked or removed by
            // another window, another app on the same vault or a restore is seen at once. Any read failure refuses.
            let binding = match keys.mcp_binding(id.to_owned()).await {
                Ok(Some(binding)) => binding,
                Ok(None) => return Err(McpRefusal::Unknown),
                Err(e) if vw_keys::is_locked(&e) => return Err(McpRefusal::Locked),
                Err(_) => return Err(McpRefusal::NotAllowed),
            };
            if shared.connectors.mcp_keys.grant_word(&binding) == "none" {
                return Err(McpRefusal::NotAllowed);
            }
            // The MCP proxy has no originating session/project. Never is conservative across project records;
            // Allowed cannot be promoted to a global grant and still needs this server's existing grant.
            if keys
                .default_entry_permission(&binding.secret_id)
                .await
                .map_err(|_| McpRefusal::PermissionsUnreadable)?
                .0
                == vw_keys::EntryPermission::Never
                || keys
                    .entry_permissions_for(&binding.secret_id)
                    .await
                    .map_err(|_| McpRefusal::PermissionsUnreadable)?
                    .iter()
                    .any(|p| p.permission == vw_keys::EntryPermission::Never)
            {
                return Err(McpRefusal::NotAllowed);
            }
            let key = keys
                .secret_value(binding.secret_id.clone(), None)
                .await
                .map_err(|e| {
                    if vw_keys::is_locked(&e) {
                        McpRefusal::Locked
                    } else {
                        McpRefusal::SecretMissing
                    }
                })?;
            if keys
                .default_entry_permission(&binding.secret_id)
                .await
                .map_err(|_| McpRefusal::PermissionsUnreadable)?
                .0
                == vw_keys::EntryPermission::Never
                || keys
                    .entry_permissions_for(&binding.secret_id)
                    .await
                    .map_err(|_| McpRefusal::PermissionsUnreadable)?
                    .iter()
                    .any(|p| p.permission == vw_keys::EntryPermission::Never)
            {
                return Err(McpRefusal::NotAllowed);
            }
            // Re-read after every awaited secret/policy check: the earlier binding cannot prove that a grant
            // still holds. A changed destination or secret also refuses this request rather than using stale data.
            let current = match keys.mcp_binding(id.to_owned()).await {
                Ok(Some(binding)) => binding,
                Ok(None) => return Err(McpRefusal::Unknown),
                Err(e) if vw_keys::is_locked(&e) => return Err(McpRefusal::Locked),
                Err(_) => return Err(McpRefusal::NotAllowed),
            };
            if current != binding || !shared.connectors.mcp_keys.permit(&current) {
                return Err(McpRefusal::NotAllowed);
            }
            if crate::agent_access::locked(&shared) {
                return Err(McpRefusal::AgentAccessLocked);
            }
            let value = if binding.bearer {
                Zeroizing::new(format!("Bearer {}", key.expose()))
            } else {
                Zeroizing::new(key.expose().to_owned())
            };
            shared.connectors.mcp_keys.count(&binding.id);
            Ok(McpTarget {
                upstream: binding.upstream,
                header: binding.header,
                value,
            })
        })
    }
}

/// Before the server starts: points Vibe Work's forwarder entries at this run's proxy port (`mcp_keys_config`).
pub(crate) fn prepare_config(home: &std::path::Path, keys: Option<&Arc<KeyService>>) {
    let Some(keys) = keys else {
        return;
    };
    match crate::mcp_keys_config::rewrite_ports(home, keys.proxy_base()) {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            entries = n,
            "mcp key servers: forwarder port updated in Vibe's config"
        ),
        Err(error) => tracing::warn!(%error, "mcp key servers: Vibe's config not updated"),
    }
}

/// Core start: registers the credentials of Vibe Work's own Vibe config as stored secrets and installs
/// the resolver in the key service's proxy.
pub(crate) fn at_start(shared: &Arc<Shared>) {
    crate::connectors_config::register_at_start(&shared.instructions.own_home);
    install(shared);
}

/// Installs the resolver in the key service's proxy.
fn install(shared: &Arc<Shared>) {
    if let Some(keys) = shared.config.keys.as_ref() {
        keys.set_mcp_resolver(Arc::new(Resolver {
            shared: Arc::downgrade(shared),
        }));
    }
}

/// Removes the resolver (core shutdown): forwarded requests answer 503 until a core installs one again.
pub(crate) fn uninstall(shared: &Shared) {
    if let Some(keys) = shared.config.keys.as_ref() {
        keys.clear_mcp_resolver();
    }
}

#[cfg(test)]
mod kg1_tests {
    use super::consent_summary;

    #[test]
    fn mcp_secret_summary_names_full_destination() {
        let address = format!("https://example.test/{}", "long/".repeat(100));
        let text = consent_summary("server", &address, "X-Secret", "Project key");
        for fact in ["server", address.as_str(), "X-Secret", "Project key"] {
            assert!(text.contains(fact), "missing {fact}");
        }
    }
}
