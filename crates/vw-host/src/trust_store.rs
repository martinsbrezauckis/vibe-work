//! Vibe Work keeps Vibe on the operating system's certificate store: its own `VIBE_HOME/config.toml` gets
//! `enable_system_trust_store = true` unless it already sets that key.
//!
//! Why: Vibe 3.1.4's default enables the system store, but its HTTP and bundled-harness builders add the pinned
//! `certifi` package's roots to it. Vibe Work's launcher guard keeps the system-store choice on the OS roots alone.
//! A user cannot update one package of Vibe's exact pins, and Vibe checks no revocation, so a bundle that still carries a revoked
//! root keeps trusting it until the next Vibe release (certifi 2026.7.22 in Vibe 3.0.2 carried one, advisory
//! GHSA-6q48-6c7w-9gw2; the 2026.10.19 bundle in 3.0.3 dropped it). The operating systems' stores drop such roots
//! with their own updates. With the key true every one of those clients verifies through the OS
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
//! the edit is verified (the new file parses and equals the old data plus this one key) before a temp-file rename, together with the other startup fixes
//! ([`crate::startup_config`]).

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
/// Goes through the guarded startup write ([`crate::startup_config`]): never through a link, never a file over 1 MiB
/// or one that changed meanwhile.
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<TrustStore, String> {
    let mut draft = crate::startup_config::Draft::open(vibe_home)
        .map_err(|e| format!("{}: {e}", vibe_home.join("config.toml").display()))?;
    let staged = draft.apply(|text, _| edit(text), TrustStore::Skipped);
    draft.commit().settle(staged, TrustStore::Skipped)
}

/// This step's edit for [`crate::startup_config`]: [`with_default_on`], or our missing notes restored beside our own value.
pub(crate) fn edit(existing: &str) -> (Option<String>, TrustStore) {
    match with_default_on(existing) {
        Ok(text) => (Some(text), TrustStore::Added),
        Err(outcome) => {
            let notes = if matches!(outcome, TrustStore::Kept(true)) {
                crate::config_comments::restored(existing, None, KEY, &NOTE)
            } else {
                None
            };
            (notes, outcome)
        }
    }
}
