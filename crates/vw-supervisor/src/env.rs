//! The server's environment: a whitelist of inherited variables, explicit variables from our own
//! code, and the API key from an injected secret provider.
//!
//! Depends on: std. Called by: `spawn` (once per server start; the key is fetched each time, so a
//! rotated key applies after a restart).
//! Invariants:
//! - The child starts from an empty environment (`env_clear`); only [`INHERITED`] names are
//!   copied from the parent, so a hostile parent env (`LD_PRELOAD`, `BASH_ENV`,...) never reaches the
//!   server or the agent tools it runs. The shared agent credential policy runs after explicit additions.
//! - A proxy in the child's environment (inherited or explicit) always comes with this computer's addresses on its
//!   `NO_PROXY` ([`LOOPBACK_NO_PROXY`]): Vibe reaches Vibe Work's model proxy at `127.0.0.1` and would otherwise send
//!   that request to the company proxy.
//! - Loader and runtime hijack variables ([`is_hijack_var`]) are refused in explicit inputs. The
//!   builder alone sets `BASH_ENV` to the app-owned startup file, prepared at each server launch.
//! - The key travels only as `MISTRAL_API_KEY` in the child env, never in argv, logs or `Debug`. The MCP forwarder
//!   token travels only as [`MCP_TOKEN_VAR`], set from the secret provider, never from `extra`.
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

/// Environment variable that carries the Mistral key to the server.
pub const API_KEY_VAR: &str = "MISTRAL_API_KEY";

/// Environment variable that carries Vibe Work's MCP forwarder token: Vibe's config entries for MCP
/// servers that use a key name it in `api_key_env`, so Vibe sends it to the loopback forwarder, which adds the real
/// key. It is the proxy token (the same value as the placeholder [`API_KEY_VAR`]), never a key. A separate name so
/// that a Vibe CLI pointed at Vibe Work's home, whose `MISTRAL_API_KEY` is the user's real key, sends nothing.
pub const MCP_TOKEN_VAR: &str = "VW_MCP_TOKEN";

/// Names copied from the parent environment when present. Locale, terminal, proxy and CA settings
/// make HTTPS and tools work; the Linux session block reaches the keyring; the Windows block is what Python and a console child need to start.
/// Matched case-insensitively on Windows (its env names are case-insensitive).
pub const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TERM",
    "TZ",
    "TMPDIR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "no_proxy",
    "all_proxy",
    // Linux desktop session: Vibe keeps connector OAuth tokens in the OS keyring, which on Linux
    // is the Secret Service on the user's D-Bus session bus. Without these two names the server cannot
    // reach it. No new capability for the agent: same user, and the bus is at `$XDG_RUNTIME_DIR/bus` anyway.
    // Unset (and so absent) on Windows and macOS.
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    // Windows
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "COMMONPROGRAMFILES",
    "TEMP",
    "TMP",
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// Python variables that only change I/O behaviour and are allowed in explicit variables.
const SAFE_PYTHON: &[&str] = &[
    "PYTHONUNBUFFERED",
    "PYTHONIOENCODING",
    "PYTHONUTF8",
    "PYTHONDONTWRITEBYTECODE",
];

/// Hijack-class variables our own code may set, but only to exactly this value.
/// `PYTHON_KEYRING_BACKEND` names a Python class to import, so any other value is refused; the null
/// backend stops Vibe reading the OS credential store (Windows `CredRead` fails over SSH with 1312,
/// and our key arrives through the environment anyway).
const PINNED_SAFE: &[(&str, &str)] = &[("PYTHON_KEYRING_BACKEND", "keyring.backends.null.Keyring")];

/// True when `name=value` is one of the [`PINNED_SAFE`] exceptions (name case-insensitive, value exact).
fn is_pinned_safe(name: &str, value: &OsStr) -> bool {
    PINNED_SAFE
        .iter()
        .any(|(n, v)| name.eq_ignore_ascii_case(n) && value == OsStr::new(v))
}

/// Git variables our own code may set for isolation (test homes); every other `GIT_*` can point
/// git at hooks, ssh commands or alternate object stores and is refused.
const SAFE_GIT: &[&str] = &["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"];

/// True for loader and runtime hijack variables: dynamic-loader injection, shell startup files,
/// interpreter option and path variables, git command overrides.
#[must_use]
pub fn is_hijack_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("LD_") || upper.starts_with("DYLD_") {
        return true;
    }
    if upper.starts_with("PYTHON") {
        return !SAFE_PYTHON.contains(&upper.as_str());
    }
    if upper.starts_with("GIT_") {
        return !SAFE_GIT.contains(&upper.as_str());
    }
    matches!(
        upper.as_str(),
        "BASH_ENV"
            | "ENV"
            | "ZDOTDIR"
            | "PROMPT_COMMAND"
            | "SHELLOPTS"
            | "BASHOPTS"
            | "IFS"
            | "NODE_OPTIONS"
            | "NODE_PATH"
            | "PERL5OPT"
            | "PERL5LIB"
            | "PERLLIB"
            | "RUBYOPT"
            | "RUBYLIB"
            | "JAVA_TOOL_OPTIONS"
            | "_JAVA_OPTIONS"
            | "JDK_JAVA_OPTIONS"
            | "UV_PYTHON"
            | "VIRTUAL_ENV"
            | "CONDA_PREFIX"
    )
}

/// The Mistral API key. `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key value (surrounding whitespace removed).
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let key = value.into().trim().to_owned();
        // a stored secret (the real key in a development run, the proxy token in the app).
        vw_secrets::register(vw_secrets::SecretKind::MistralKey, &key);
        Self(key)
    }

    /// The raw value, for the child environment and the stderr redaction list only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Failure to obtain the key.
#[derive(Debug, thiserror::Error)]
#[error("could not read the Mistral API key: {0}")]
pub struct SecretError(pub String);

/// Supplies the key at each server start (keychain in the app, environment in tests).
pub trait SecretProvider: Send + Sync {
    /// The key, or `None` to start without one (no-key handshake, local-only reads).
    ///
    /// # Errors
    /// [`SecretError`] when a configured store fails.
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError>;

    /// The token Vibe sends to Vibe Work's MCP forwarder, as [`MCP_TOKEN_VAR`]; asked right after
    /// [`SecretProvider::mistral_api_key`] at the same start, so a provider that rotates a token there returns
    /// the new one. Default: none (the variable is not set).
    fn mcp_forward_token(&self) -> Option<ApiKey> {
        None
    }
}

/// No key: the server starts unauthenticated.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSecret;

impl SecretProvider for NoSecret {
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError> {
        Ok(None)
    }
}

/// Reads `MISTRAL_API_KEY` from this process's environment (tests and dev runs).
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn mistral_api_key(&self) -> Result<Option<ApiKey>, SecretError> {
        Ok(std::env::var(API_KEY_VAR)
            .ok()
            .map(ApiKey::new)
            .filter(|key| !key.expose().is_empty()))
    }
}

/// An explicit variable was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvError {
    /// The name is a loader/runtime hijack variable.
    #[error("refusing to pass {0} to the server: loader/runtime hijack variable")]
    Hijack(String),
    /// The key (and the MCP forwarder token) may only come from the secret provider.
    #[error(
        "refusing {API_KEY_VAR} or {MCP_TOKEN_VAR} as an explicit variable: they come from the secret provider"
    )]
    KeyAsExtra,
}

/// Inputs for one child environment.
pub struct EnvPlan<'a> {
    /// The parent environment to filter (normally `std::env::vars_os()`).
    pub parent: Vec<(OsString, OsString)>,
    /// Explicit variables from our own code (validated).
    pub extra: &'a [(String, OsString)],
    /// `VIBE_HOME` for the server.
    pub vibe_home: &'a Path,
    /// `HOME` override (test isolation); `None` keeps the inherited one.
    pub home: Option<&'a Path>,
    /// The key, if any.
    pub key: Option<&'a ApiKey>,
    /// The MCP forwarder token, if any.
    pub mcp_token: Option<&'a ApiKey>,
}

/// Builds the child environment from `plan`.
///
/// # Errors
/// [`EnvError`] when an explicit variable is refused.
pub fn build_env(plan: &EnvPlan<'_>) -> Result<Vec<(OsString, OsString)>, EnvError> {
    for (name, value) in plan.extra {
        if name.eq_ignore_ascii_case(API_KEY_VAR) || name.eq_ignore_ascii_case(MCP_TOKEN_VAR) {
            return Err(EnvError::KeyAsExtra);
        }
        if is_hijack_var(name) && !is_pinned_safe(name, value) {
            return Err(EnvError::Hijack(name.clone()));
        }
    }
    let mut env: Vec<(OsString, OsString)> = plan
        .parent
        .iter()
        .filter(|(name, _)| inherited(name))
        .cloned()
        .collect();
    let mut set = |name: &str, value: OsString| {
        env.retain(|(existing, _)| !same_name(existing, OsStr::new(name)));
        env.push((OsString::from(name), value));
    };
    for (name, value) in plan.extra {
        set(name, value.clone());
    }
    set("VIBE_HOME", plan.vibe_home.as_os_str().to_owned());
    set(
        "BASH_ENV",
        crate::shell_env::path(plan.vibe_home).into_os_string(),
    );
    // The shell tool uses SHELL on Unix; a user's login shell may not read BASH_ENV.
    #[cfg(unix)]
    set("SHELL", OsString::from("/bin/bash"));
    if let Some(home) = plan.home {
        set("HOME", home.as_os_str().to_owned());
        if cfg!(windows) {
            set("USERPROFILE", home.as_os_str().to_owned());
        }
    }
    if let Some(key) = plan.key {
        set(API_KEY_VAR, OsString::from(key.expose()));
    }
    if let Some(token) = plan.mcp_token {
        set(MCP_TOKEN_VAR, OsString::from(token.expose()));
    }
    keep_loopback_off_the_proxy(&mut env);
    crate::isolate_credentials(&mut env);
    Ok(env)
}

/// Addresses of this computer, in the no-proxy list Vibe gets when it has a proxy. Vibe reaches Vibe Work's model proxy
/// at `127.0.0.1` and its HTTP client has no implicit loopback bypass (probed on Vibe 2.26.1: with `HTTP_PROXY` set and
/// no `NO_PROXY`, a request to `http://127.0.0.1:<port>` went to the proxy), so a proxy in the environment without these
/// entries would send every model request of Vibe to the company proxy. Same list as `vw_mistral::network::LOOPBACK`
/// (a private test keeps the two equal; this crate does not depend on the HTTP stack).
pub const LOOPBACK_NO_PROXY: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// When the child environment names a proxy (`HTTP_PROXY`, `HTTPS_PROXY` or `ALL_PROXY`, either case), makes both
/// spellings of `NO_PROXY` hold the list it already has (Python reads the lower-case one first) plus this computer's
/// addresses. Without a proxy nothing is touched.
fn keep_loopback_off_the_proxy(env: &mut Vec<(OsString, OsString)>) {
    let value = |env: &[(OsString, OsString)], name: &str| {
        env.iter()
            .find(|(existing, _)| same_name(existing, OsStr::new(name)))
            .and_then(|(_, value)| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
    };
    let proxied = [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ]
    .iter()
    .any(|name| value(env, name).is_some());
    if !proxied {
        return;
    }
    let current = value(env, "no_proxy")
        .or_else(|| value(env, "NO_PROXY"))
        .unwrap_or_default();
    let mut entries: Vec<&str> = current
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    for local in LOOPBACK_NO_PROXY {
        if !entries
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(local))
        {
            entries.push(local);
        }
    }
    let merged = OsString::from(entries.join(","));
    for name in ["NO_PROXY", "no_proxy"] {
        env.retain(|(existing, _)| !same_name(existing, OsStr::new(name)));
        env.push((OsString::from(name), merged.clone()));
    }
}

fn inherited(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    !is_hijack_var(name)
        && INHERITED
            .iter()
            .any(|allowed| same_name(OsStr::new(allowed), OsStr::new(name)))
}

fn same_name(a: &OsStr, b: &OsStr) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}
