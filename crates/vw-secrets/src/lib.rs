//! The stored secret values of this process, so the Debug API can prove it never returns one.
//!
//! the Debug API controls the whole app as the user, "you can't get secrets from the store
//! as values". Everything else the app shows is open to it, so the secret rule needs a check that does not depend
//! on every route being written carefully: each place where a stored secret enters memory registers its value
//! here ([`register`]), and the Debug API's response layer replaces every registered value in every answer
//! ([`scrub`]) before it leaves the process.
//!
//! Registered (callers), each when the value is stored or read from storage, never for an unconfirmed input (a
//! wrong passphrase, a key that failed its test): the Mistral API key read from the vault and the remembered-device
//! secret (`vw-vault`), every stored vault value each time the key service finds the vault open (start with a
//! kept or remembered key, create, unlock: `vw-keys` `scan_secrets::open_and_registered`, so a value is hidden before
//! anything lists it), a key or passphrase once `setupStoreKey`, `createVault` or `unlockVault` succeeded
//! (`vw-core-api` `keys`), keys found in Vibe's own `.env` / keyring when the user asks to detect them
//! (`vw-keys`), the key-proxy token (`vw-keyproxy`), the Debug API token at start (`vw-debug-api`), the
//! environment key of a development run (`vw-supervisor`), the credentials of MCP servers in Vibe Work's own
//! config at core start (`vw-core-api` `connectors_config`: header values, environment values, URL user info), and
//! the key the MCP forwarder inserts for a server (`vw-keyproxy`), whose answers it scrubs with [`StreamScrub`].
//!
//! What this is: a convenience that hides known values where it can, not a guarantee. A value the client
//! transforms (reversed, hex-dumped in a terminal, re-encoded) is not recognised; with the Debug API on, the client
//! can do what the user can do in the app (`docs/public/DEBUG_API.md`).
//!
//! Depends on: memchr, base64, zeroize. Called by: the crates above (register), vw-debug-api (find).
//! Invariants:
//! - A value shorter than [`MIN_LEN`] bytes is not registered (a short value would match ordinary text in every
//!   answer); a caller that knows the one value it handed to an answer's sender withholds it in that answer at any
//!   length ([`StreamScrub::with_value`], [`holds`]). Nothing is ever removed during a run: a rotated-away key is
//!   still a secret, and a full registry never evicts: past [`MAX_ENTRIES`] - [`PINNED_RESERVE`] entries only the
//!   pinned kinds ([`SecretKind::pinned`]: the Mistral key, vault values, the Debug API and proxy tokens) are still
//!   registered; past [`MAX_ENTRIES`] nothing is, and a warning names the kind (never the value).
//! - Each value is searched in the forms it can take inside an answer: as it is, JSON-escaped, percent-encoded,
//!   hex (lower and upper case), and inside standard or URL-safe base64 at each of the three byte
//!   alignments ([`forms`]). Not covered (stated, not hidden): other encodings (gzip, a hex dump with line breaks or
//!   spaces), a value split across two answers or lines (one answer that arrives in parts is
//!   covered by [`StreamScrub`]), a reversed or otherwise transformed value, and pixels of a
//!   screenshot.
//! - [`scrub`] replaces each occurrence of any form with [`marker`] (plain ASCII without quotes or backslashes, so
//!   a JSON text stays valid JSON) and reports the kinds it found; the answer stays usable (a session whose agent
//!   printed the proxy token is still readable) and holds no value.
//! - The stored forms are wiped from memory when the registry is dropped; the value itself is never logged or
//!   returned by this crate (only kinds are).

use std::fmt::Write as _;
use std::sync::{OnceLock, PoisonError, RwLock};

mod stream;
pub use stream::StreamScrub;

use base64::Engine as _;
use zeroize::Zeroizing;

/// Shortest value that is registered (bytes).
pub const MIN_LEN: usize = 8;
/// Most values kept; past it a new value is refused, nothing is evicted (a run never comes close).
pub const MAX_ENTRIES: usize = 4096;
/// Entries kept free for the pinned kinds: other kinds are refused once only this many places are left.
pub const PINNED_RESERVE: usize = 64;

/// What a registered value is, for the refusal and the log line (never the value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    /// The Mistral API key (vault, Vibe's own copy, or the environment).
    MistralKey,
    /// Any value read from the vault through Vibe Work's client.
    VaultValue,
    /// A credential of an MCP server or connector (header, environment value, URL user info).
    ConnectorCredential,
    /// The Debug API's bearer token.
    DebugApiToken,
    /// The local model proxy's token.
    ProxyToken,
    /// A passphrase or another secret the user typed.
    Typed,
}

impl SecretKind {
    /// The stored secrets that keep a place even when other kinds fill the registry (module docs).
    #[must_use]
    pub fn pinned(self) -> bool {
        matches!(
            self,
            Self::MistralKey | Self::VaultValue | Self::DebugApiToken | Self::ProxyToken
        )
    }

    /// Wire name (`mistralKey`,...), used in the Debug API's refusal.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MistralKey => "mistralKey",
            Self::VaultValue => "vaultValue",
            Self::ConnectorCredential => "connectorCredential",
            Self::DebugApiToken => "debugApiToken",
            Self::ProxyToken => "proxyToken",
            Self::Typed => "typed",
        }
    }
}

struct Entry {
    kind: SecretKind,
    /// The value as it is (for de-duplication) and every searched form.
    raw: Zeroizing<Vec<u8>>,
    forms: Vec<Zeroizing<Vec<u8>>>,
}

fn registry() -> &'static RwLock<Vec<Entry>> {
    static REGISTRY: OnceLock<RwLock<Vec<Entry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Remembers `value` as a stored secret of `kind`. Shorter than [`MIN_LEN`] or already known: no-op. A full
/// registry refuses it with a warning (module docs); nothing registered before is ever removed.
pub fn register(kind: SecretKind, value: &str) {
    if value.len() < MIN_LEN {
        return;
    }
    let mut entries = registry().write().unwrap_or_else(PoisonError::into_inner);
    if entries.iter().any(|e| e.raw.as_slice() == value.as_bytes()) {
        return;
    }
    let limit = if kind.pinned() {
        MAX_ENTRIES
    } else {
        MAX_ENTRIES - PINNED_RESERVE
    };
    if entries.len() >= limit {
        tracing::warn!(
            kind = kind.as_str(),
            entries = entries.len(),
            "the secret registry is full: this value is not hidden from Debug API answers"
        );
        return;
    }
    entries.push(Entry {
        kind,
        raw: Zeroizing::new(value.as_bytes().to_vec()),
        forms: forms(value).into_iter().map(Zeroizing::new).collect(),
    });
}

/// The kind of the first registered value found in `haystack` in any of its forms, or `None`.
#[must_use]
pub fn find(haystack: &[u8]) -> Option<SecretKind> {
    let entries = registry().read().unwrap_or_else(PoisonError::into_inner);
    entries
        .iter()
        .find(|e| {
            e.forms
                .iter()
                .any(|form| memchr::memmem::find(haystack, form).is_some())
        })
        .map(|e| e.kind)
}

/// True when `haystack` holds `value` in any of its forms ([`forms`], at any length: [`exact_forms`]), whether or
/// not `value` is registered (a check of one known value, for text that is refused rather than scrubbed).
#[must_use]
pub fn holds(haystack: &[u8], value: &str) -> bool {
    exact_forms(value)
        .iter()
        .any(|form| memchr::memmem::find(haystack, form).is_some())
}

/// The text that replaces a value of `kind`: `[secret withheld: mistralKey]`.
#[must_use]
pub fn marker(kind: SecretKind) -> String {
    format!("[secret withheld: {}]", kind.as_str())
}

/// `haystack` with every form of every registered value replaced by its [`marker`], and the kinds found
/// (first occurrence order, no duplicates). `None` when nothing was found (the caller keeps its bytes).
#[must_use]
pub fn scrub(haystack: &[u8]) -> Option<(Vec<u8>, Vec<SecretKind>)> {
    let entries = registry().read().unwrap_or_else(PoisonError::into_inner);
    let mut current: Option<Vec<u8>> = None;
    let mut kinds: Vec<SecretKind> = Vec::new();
    for entry in entries.iter() {
        for form in &entry.forms {
            let text = current.as_deref().unwrap_or(haystack);
            if memchr::memmem::find(text, form).is_none() {
                continue;
            }
            current = Some(replace(text, form, marker(entry.kind).as_bytes()));
            if !kinds.contains(&entry.kind) {
                kinds.push(entry.kind);
            }
        }
    }
    current.map(|bytes| (bytes, kinds))
}

/// `text` with every occurrence of `form` replaced by `replacement`.
pub(crate) fn replace(text: &[u8], form: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    for found in memchr::memmem::find_iter(text, form) {
        if found < at {
            continue;
        }
        out.extend_from_slice(&text[at..found]);
        out.extend_from_slice(replacement);
        at = found + form.len();
    }
    out.extend_from_slice(&text[at..]);
    out
}

/// The length of the longest end of `text` that is the start (never the whole) of a registered value's form: what a
/// [`StreamScrub`] keeps until the next part arrives. 0 when no end of `text` can start one.
pub(crate) fn pending_prefix(text: &[u8]) -> usize {
    let entries = registry().read().unwrap_or_else(PoisonError::into_inner);
    pending_in(text, entries.iter().flat_map(|e| e.forms.iter()))
}

/// [`pending_prefix`] for the given forms.
pub(crate) fn pending_in<'a>(
    text: &[u8],
    forms: impl Iterator<Item = &'a Zeroizing<Vec<u8>>>,
) -> usize {
    let mut keep = 0;
    for form in forms {
        let longest = form.len().saturating_sub(1).min(text.len());
        for n in (keep + 1..=longest).rev() {
            let start = text.len() - n;
            if text[start] == form[0] && text[start..] == form[..n] {
                keep = n;
                break;
            }
        }
    }
    keep
}

/// How many values are registered (tests and the startup log; never the values).
#[must_use]
pub fn count() -> usize {
    registry()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
}

/// Every form of `value` that is searched (module docs), without duplicates.
#[must_use]
pub fn forms(value: &str) -> Vec<Vec<u8>> {
    forms_from(value, MIN_LEN)
}

/// [`forms`] without the registry's minimum length: the forms of one value a caller knows it handed out (the MCP
/// forwarder's inserted key), searched in that one answer only, never in every answer like the registry's.
#[must_use]
pub fn exact_forms(value: &str) -> Vec<Vec<u8>> {
    forms_from(value, 1)
}

fn forms_from(value: &str, min: usize) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = vec![value.as_bytes().to_vec()];
    let mut add = |form: Vec<u8>| {
        if form.len() >= min && !out.contains(&form) {
            out.push(form);
        }
    };
    add(json_escaped(value).into_bytes());
    add(percent_encoded(value).into_bytes());
    let hex = hex::encode(value);
    add(hex.to_ascii_uppercase().into_bytes());
    add(hex.into_bytes());
    let bytes = value.as_bytes();
    for skip in 0..3 {
        let Some(rest) = bytes.get(skip..) else {
            break;
        };
        // Only whole 3-byte groups: their encoding is the same wherever the group sits in a longer input.
        let whole = rest.len() / 3 * 3;
        let Some(groups) = rest.get(..whole) else {
            continue;
        };
        if groups.is_empty() {
            continue;
        }
        add(base64::engine::general_purpose::STANDARD_NO_PAD
            .encode(groups)
            .into_bytes());
        add(base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(groups)
            .into_bytes());
    }
    out
}

/// `value` as it appears inside a JSON string (the escapes of `serde_json` and `JSON.stringify`).
fn json_escaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// `value` percent-encoded (every byte outside the URL's unreserved set).
fn percent_encoded(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}
