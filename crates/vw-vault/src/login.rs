//! Website logins: the vault's plain `login` items, the ones the Vault app's panel creates and ShellX's
//! browser fills from, so a login stored once works in every app.
//!
//! Format: Vault's item JSON (`items::VaultItem`, upstream `vault-client/src/items.rs:37-56`): `type = "login"`,
//! `title` (the name), `username`, `password`, `url` (the site), `notes`. Vibe Work's own logins have the id
//! `vibe-work-login-<16 hex>` and one extra field `createdBy = "vibe-work"` (every Vault client keeps unknown fields).
//! Which logins exist for Vibe Work: every plain login whose `url` is an `http`/`https` address with a host and no user
//! info ([`from_item`]); Vibe Work removes only its own ([`is_own`]). Which login fits a page: the exact origin only
//! ([`fits`]; upstream `domain_matches`, `items.rs:253-264`, also accepts subdomains: not taken).
//!
//! Depends on: `items`, `secret`, vault-core (random ids), url, vw-secrets. Called by: vw-keys (`logins`).
//! Invariants:
//! - [`Login`] (what lists return) never holds the password; the password leaves this module only through
//!   [`password_of`], as a [`SecretString`] registered as a stored secret first, and only for a password
//!   of at least [`MIN_PASSWORD`] bytes (shorter values cannot be hidden from answers, `vw_secrets::MIN_LEN`).
//! - Only items with the exact shape above parse as logins; anything else is ignored, never guessed.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::items::VaultItem;
use crate::secret::SecretString;

/// Id prefix of the logins Vibe Work creates.
pub const ID_PREFIX: &str = "vibe-work-login-";
/// Shortest password Vibe Work fills or stores (bytes): the secret registry hides only values this long.
pub const MIN_PASSWORD: usize = 8;
/// Longest password (bytes).
pub const MAX_PASSWORD: usize = 4096;
/// Longest user name (characters).
pub const MAX_USERNAME: usize = 200;
/// Longest site address (characters).
pub const MAX_URL: usize = 2000;

/// A login (metadata only: never the password).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    /// Item id.
    pub id: String,
    /// Its name (the item title).
    pub name: String,
    /// The site address as stored.
    pub url: String,
    /// The site's origin (`https://example.com`).
    pub origin: String,
    /// The site's host (lower case).
    pub host: String,
    /// The user name (not a secret: the agent may see it).
    pub username: String,
    /// The item has a password.
    pub has_password: bool,
    /// Created by Vibe Work (it may remove it).
    pub own: bool,
    /// Last change, ms.
    pub updated_ms: i64,
}

/// True when `id` is a login Vibe Work created.
#[must_use]
pub fn is_own(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// A new random id for a login.
#[must_use]
pub fn new_id() -> String {
    format!(
        "{ID_PREFIX}{}",
        hex::encode(vault_core::random_bytes::<8>())
    )
}

/// The site of `address` as (origin, host): an `http`/`https` URL with a host and no user info. A bare host name
/// (`example.com/login`) is read as `https://`.
#[must_use]
pub fn site_of(address: &str) -> Option<(String, String)> {
    let address = address.trim();
    if address.is_empty() || address.chars().count() > MAX_URL {
        return None;
    }
    let parsed = Url::parse(address)
        .ok()
        .filter(Url::has_host)
        .or_else(|| Url::parse(&format!("https://{address}")).ok())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some((parsed.origin().ascii_serialization(), host))
}

/// True for a host on this computer (`localhost`, `127.0.0.0/8`, `[::1]`): any local program, the agent's own servers
/// included, can listen there.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// True when a page at `page_origin` may use `login`: only the login's exact origin (scheme, host and port).
/// A review found: Vault's `domain_matches` also accepted every subdomain, which lets a login saved
/// for a parent fill on a user-content subdomain (`someone.github.io`, `*.pages.dev`); subdomains may come back later
/// with a public-suffix-list check. On this computer the port matters too (any local program can listen there).
#[must_use]
pub fn fits(login: &Login, page_origin: &str) -> bool {
    login.origin.eq_ignore_ascii_case(page_origin)
}

/// The login stored in `item`, if it is one (module docs).
#[must_use]
pub fn from_item(item: &VaultItem) -> Option<Login> {
    if item.kind != "login" {
        return None;
    }
    let (origin, host) = site_of(&item.url)?;
    Some(Login {
        id: item.id.clone(),
        name: if item.title.trim().is_empty() {
            host.clone()
        } else {
            item.title.trim().to_owned()
        },
        url: item.url.clone(),
        origin,
        host,
        username: item.username.chars().take(MAX_USERNAME).collect(),
        has_password: !item.password.is_empty(),
        own: is_own(&item.id)
            && item.extra.get("createdBy").and_then(Value::as_str) == Some("vibe-work"),
        updated_ms: item.updated_ms,
    })
}

/// The password of the login in `item`, exactly as stored, registered as a stored secret. `None` when `item` is not a
/// login, has no password, or one shorter than [`MIN_PASSWORD`] or longer than [`MAX_PASSWORD`] bytes.
#[must_use]
pub fn password_of(item: &VaultItem) -> Option<SecretString> {
    from_item(item)?;
    let length = item.password.len();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&length) {
        return None;
    }
    let value = SecretString::exact(item.password.clone());
    value.register_as(vw_secrets::SecretKind::VaultValue);
    Some(value)
}

/// The exact stored password of any listed login for a user-initiated clipboard copy. Foreign Vault clients may
/// have shorter passwords than Vibe Work permits for browser filling; this path never returns the value to a client.
#[must_use]
pub fn copy_password_of(item: &VaultItem) -> Option<SecretString> {
    from_item(item)?;
    if item.password.is_empty() {
        return None;
    }
    let value = SecretString::exact(item.password.clone());
    value.register_as(vw_secrets::SecretKind::VaultValue);
    Some(value)
}

/// What the user typed for a new login.
pub struct NewLogin<'a> {
    /// The name (empty: the site's host).
    pub name: &'a str,
    /// The site address.
    pub url: &'a str,
    /// The user name.
    pub username: &'a str,
    /// The password.
    pub password: &'a SecretString,
}

/// The item for a new or replaced own login (`id` from [`new_id`] or an existing own id). `None` when the address is
/// not a site or a field is out of bounds.
#[must_use]
pub fn item(
    id: &str,
    new: &NewLogin<'_>,
    created_ms: Option<i64>,
    now_ms: i64,
) -> Option<VaultItem> {
    let (_, host) = site_of(new.url)?;
    let length = new.password.len();
    if !(MIN_PASSWORD..=MAX_PASSWORD).contains(&length)
        || new.username.chars().count() > MAX_USERNAME
        || new.name.chars().count() > 80
    {
        return None;
    }
    let mut extra = BTreeMap::new();
    extra.insert(
        "createdBy".to_owned(),
        Value::String("vibe-work".to_owned()),
    );
    Some(VaultItem {
        id: id.to_owned(),
        kind: "login".to_owned(),
        title: if new.name.trim().is_empty() {
            host
        } else {
            new.name.trim().to_owned()
        },
        username: new.username.trim().to_owned(),
        password: new.password.expose().to_owned(),
        url: new.url.trim().to_owned(),
        notes: String::new(),
        created_ms: created_ms.unwrap_or(now_ms),
        updated_ms: now_ms,
        extra,
    })
}
