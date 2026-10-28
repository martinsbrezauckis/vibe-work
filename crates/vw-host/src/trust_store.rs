//! Vibe trusts the operating system's certificate store by default: Vibe Work's own `VIBE_HOME/config.toml` gets
//! `enable_system_trust_store = true` unless it already sets that key.
//!
//! Why: while the key is false (Vibe's default, `vibe_schema.py:867`), Vibe's HTTPS clients (MCP servers over HTTP,
//! MCP sign-in, the skills registry, connectors) trust only the certificate bundle of the `certifi` package Vibe
//! pins (`utils/http.py:47-55`; the runtime's `_ssl.py:21-35` adds it on top of the platform roots). Vibe 3.0.2 pins
//! certifi 2026.7.22, which still carries a root whose intermediate keys are compromised and revoked (advisory
//! GHSA-6q48-6c7w-9gw2); Vibe checks no revocation, and a user cannot update one package of Vibe's exact pins. The
//! operating systems' stores dropped that root. With the key true every one of those clients verifies through the OS
//! store (`truststore`; on Linux it reads the distribution's CA files), and a company's own CA in that store works
//! too. Model calls are not affected either way: they go through Vibe Work's model proxy.
//!
//! The user's own setting keeps working: a config that already sets `enable_system_trust_store` (true or false, the
//! user's edit or ours from an earlier start) is left as it is. The window and the agent cannot write the key (the
//! gate's config rules refuse it), so only a hand edit of the file sets it to false.
//!
//! Depends on: `toml` (parsing only; the file is edited as text so the user's comments and layout stay).
//! Called by: `launch` (product and development launches, before the server starts), tests.
//! Invariants: only Vibe Work's own `VIBE_HOME` (never `~/.vibe`); a file that is not valid TOML is left byte-identical;
//! the edit is verified (the new file parses and equals the old data plus this one key) before a temp-file rename.

use std::path::Path;

use toml::{Table, Value};

/// The key (top level of Vibe's config).
pub const KEY: &str = "enable_system_trust_store";

/// The lines written above the key (English, no paths; they ship into the user's config).
const NOTE: [&str; 2] = [
    "# Vibe Work: Vibe checks HTTPS certificates with your system's certificate store, not the copy bundled with",
    "# Vibe's Python packages, which can lag behind removed certificate authorities.",
];

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStore {
    /// `enable_system_trust_store = true` was added.
    Added,
    /// The config already sets the key (to this value): left as it is.
    Kept(bool),
    /// Left unchanged, with the reason.
    Skipped(String),
}

/// The config text with `enable_system_trust_store = true` added at the top (top-level keys must come before the
/// first table), or the outcome when nothing is to be written. Pure.
///
/// # Errors
/// `Kept` / `Skipped` when the text stays as it is.
pub fn with_default_on(existing: &str) -> Result<String, TrustStore> {
    let before: Table = existing.parse().map_err(|_| {
        TrustStore::Skipped(
            "config.toml is not valid TOML; Vibe reports the error itself".to_owned(),
        )
    })?;
    if let Some(value) = before.get(KEY) {
        return Err(value.as_bool().map_or_else(
            || TrustStore::Skipped(format!("{KEY} is not true or false")),
            TrustStore::Kept,
        ));
    }
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = String::with_capacity(existing.len() + 250);
    for line in NOTE {
        out.push_str(line);
        out.push_str(newline);
    }
    out.push_str(KEY);
    out.push_str(" = true");
    out.push_str(newline);
    if !existing.is_empty() {
        out.push_str(newline);
        out.push_str(existing);
    }
    let Ok(mut after) = out.parse::<Table>() else {
        return Err(TrustStore::Skipped(
            "the edit could not be verified".to_owned(),
        ));
    };
    if after.remove(KEY) != Some(Value::Boolean(true)) || after != before {
        return Err(TrustStore::Skipped(
            "the edit could not be verified".to_owned(),
        ));
    }
    Ok(out)
}

/// Makes `vibe_home/config.toml` set `enable_system_trust_store` (true unless already set). A missing file is created.
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<TrustStore, String> {
    let path = vibe_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let updated = match with_default_on(&existing) {
        Ok(text) => text,
        Err(outcome) => return Ok(outcome),
    };
    std::fs::create_dir_all(vibe_home).map_err(|e| format!("{}: {e}", vibe_home.display()))?;
    let tmp = vibe_home.join("config.toml.vw-tmp");
    std::fs::write(&tmp, &updated).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    Ok(TrustStore::Added)
}
