//! The address rule for MCP servers added by URL, and the typed check of `mcp_catalog/add` / `mcp/add` that applies it.
//!
//! Depends on: `url` (WHATWG parsing), vw-protocol (`MCPAddParams` via `table::typed`). Called by: the
//! method table (the `add` rows), vw-core-api (validates and canonicalises before asking the native dialog,
//! so the user is never asked about an address the gate would refuse, and the dialog shows what is sent).
//! Invariants:
//! - **The address Vibe receives is exactly the one checked**: the WHATWG parser (`url`) and Vibe's Python `urlsplit` disagree on inputs such as
//!   `https://good.example\@evil.example/` (WHATWG: host `good.example`; Python: `evil.example`). So the
//!   check returns the parser's canonical serialisation and accepts an input only when it differs from
//!   that by trivial normalisation (case of scheme and host, a default port, the `/` of an empty path);
//!   anything else (backslashes, percent-encoded or dot-segment paths, numeric IPv4 shorthands, IDN hosts,
//!   leading-zero ports) is refused. The core sends the canonical form; the gate, which never rewrites
//!   params, accepts only the canonical form.
//! - `https://` to any host; `http://` only to this computer, decided on the PARSED host and exactly
//!   `127.0.0.1`, `[::1]` or `localhost`: Vibe's own rule (`core/config/mcp_servers.py:271-299`) minus its
//!   `allowInsecureHttp` escape, which the gate refuses outright.
//! - No user name or password, fragment or query: a key in the address would be written to Vibe's config
//!   in clear, and a user name before the host is a disguise.
//! - No whitespace, control characters (NUL included) or backslashes; at most [`MAX_MCP_URL_LENGTH`]
//!   characters.
//! - Refusal texts are fixed sentences: they never quote the address (refusals are logged and broadcast,
//!   `error.rs` invariant).

use std::net::{Ipv4Addr, Ipv6Addr};

use serde_json::Value;
use url::{Host, Url};

/// Longest accepted server address.
pub const MAX_MCP_URL_LENGTH: usize = 2048;
/// Most OAuth scopes one add may ask for.
const MAX_SCOPES: usize = 32;
/// Longest scope name.
const MAX_SCOPE_LENGTH: usize = 128;
/// Longest server name.
const MAX_NAME_LENGTH: usize = 128;

/// Why an MCP server address was refused. `Display` is a plain sentence without the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum McpUrlRefusal {
    /// Empty, too long, or holding spaces, control characters or backslashes.
    #[error(
        "the address is empty, too long, or contains spaces, control characters or backslashes"
    )]
    Malformed,
    /// Not a complete absolute address.
    #[error("the address is not a complete web address")]
    NotAUrl,
    /// A scheme other than https (or http for this computer).
    #[error("the address must start with https:// (http:// only for this computer)")]
    Scheme,
    /// Plain http to another computer: the sign-in and the agent's data would travel unencrypted.
    #[error(
        "http:// is allowed only for a server on this computer (127.0.0.1, [::1], localhost); use https://"
    )]
    InsecureRemote,
    /// A user name or password before the host.
    #[error("the address hides a user name or password before the host")]
    Credentials,
    /// A `#fragment` part.
    #[error("the address must not have a # part")]
    Fragment,
    /// A `?query` part.
    #[error(
        "addresses with a ? part are not supported yet: a key in the address would be stored in Vibe's config in clear"
    )]
    Query,
    /// No host.
    #[error("the address has no host")]
    NoHost,
    /// The address changes when read as a web address (beyond the case of scheme and host, a default port
    /// or the `/` of an empty path), so another reader could see a different address.
    #[error(
        "the address is not in its plain form (it changes when read as a web address); type it without encoded characters, ./.. parts, IP shorthands or non-ASCII host names"
    )]
    NotCanonical,
}

/// True for the hosts plain http may reach: exactly `localhost`, `127.0.0.1` or `::1` (parsed).
fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => *name == "localhost",
        Host::Ipv4(ip) => *ip == Ipv4Addr::LOCALHOST,
        Host::Ipv6(ip) => *ip == Ipv6Addr::LOCALHOST,
    }
}

/// The raw address with only the trivial normalisations applied (lower-case scheme and authority, the
/// default port dropped, `/` for an empty path). Inputs here hold no `?`, `#`, `\\`, `@` or whitespace.
fn trivially_normalised(text: &str) -> Option<String> {
    let (scheme, rest) = text.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let mut authority = authority.to_ascii_lowercase();
    let default = if scheme == "https" { ":443" } else { ":80" };
    if let Some(stripped) = authority.strip_suffix(default) {
        authority = stripped.to_owned();
    }
    Some(format!("{scheme}://{authority}{path}"))
}

/// Checks an MCP server address against the rule in the module docs and returns its canonical form, the
/// only form that may be sent to Vibe.
///
/// # Errors
/// The [`McpUrlRefusal`] that applies first (in the order: shape, parse, scheme, credentials, fragment,
/// query, host, canonical form, plain http to another computer).
pub fn check_mcp_server_url(text: &str) -> Result<String, McpUrlRefusal> {
    if text.is_empty()
        || text.chars().count() > MAX_MCP_URL_LENGTH
        || text
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(McpUrlRefusal::Malformed);
    }
    let url = Url::parse(text).map_err(|_| McpUrlRefusal::NotAUrl)?;
    let scheme = url.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(McpUrlRefusal::Scheme);
    }
    if !url.username().is_empty() || url.password().is_some() || text.contains('@') {
        return Err(McpUrlRefusal::Credentials);
    }
    if url.fragment().is_some() || text.contains('#') {
        return Err(McpUrlRefusal::Fragment);
    }
    if url.query().is_some() || text.contains('?') {
        return Err(McpUrlRefusal::Query);
    }
    let Some(host) = url.host() else {
        return Err(McpUrlRefusal::NoHost);
    };
    let canonical = url.as_str().to_owned();
    if trivially_normalised(text).as_deref() != Some(canonical.as_str()) {
        return Err(McpUrlRefusal::NotCanonical);
    }
    if scheme == "http" && !is_loopback(&host) {
        return Err(McpUrlRefusal::InsecureRemote);
    }
    Ok(canonical)
}

/// The table check of `mcp_catalog/add` and `mcp/add`: the generated type (via `typed`), then the address
/// rule, no `allowInsecureHttp: true`, a short name and a bounded list of short scopes.
pub(crate) fn check_add<M>(params: &Value) -> Result<(), String>
where
    M: vw_protocol::RequestMethod,
    M::Params: serde::de::DeserializeOwned + serde::Serialize,
{
    // URL additions use the existing mediated credential path. Vibe's optional credential,
    // command and environment fields must never bypass it, even as the wire model grows.
    // Refuse all environment names, including the forwarder token and held provider keys.
    for field in [
        "command",
        "args",
        "env",
        "headers",
        "apiKeyEnv",
        "apiKeyHeader",
        "apiKeyFormat",
        "startupTimeoutSec",
        "toolTimeoutSec",
    ] {
        if params.get(field).is_some_and(|value| !value.is_null()) {
            return Err(format!(
                "{field} is not supported here; add tool servers through Vibe Work's settings"
            ));
        }
    }
    if params.get("transport").and_then(Value::as_str) == Some("stdio") {
        return Err("command tool servers cannot be added by URL".into());
    }
    crate::table::typed::<M>(params)?;
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let canonical = check_mcp_server_url(url).map_err(|refusal| refusal.to_string())?;
    // The gate never rewrites params: only the canonical form passes (the core sends that form).
    if canonical != url {
        return Err(McpUrlRefusal::NotCanonical.to_string());
    }
    if params.get("allowInsecureHttp").and_then(Value::as_bool) == Some(true) {
        return Err(
            "allowInsecureHttp is not accepted: plain http only to this computer".to_owned(),
        );
    }
    if let Some(name) = params.get("name").and_then(Value::as_str)
        && (name.trim().is_empty()
            || name.chars().count() > MAX_NAME_LENGTH
            || name.chars().any(char::is_control))
    {
        return Err(format!(
            "the name must have 1 to {MAX_NAME_LENGTH} characters and no control characters"
        ));
    }
    if let Some(scopes) = params.get("scopes").and_then(Value::as_array) {
        let bad = scopes.iter().any(|s| {
            s.as_str().is_none_or(|s| {
                s.is_empty()
                    || s.len() > MAX_SCOPE_LENGTH
                    || s.chars().any(|c| c.is_whitespace() || c.is_control())
            })
        });
        if scopes.len() > MAX_SCOPES || bad {
            return Err(format!(
                "at most {MAX_SCOPES} scopes of 1 to {MAX_SCOPE_LENGTH} characters without spaces"
            ));
        }
    }
    Ok(())
}
