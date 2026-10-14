//! Names and addresses that come from the repository (branch, upstream and remote names, remote addresses)
//! before they reach a git or gh command line or a forge address (a remote
//! literally named `--upload-pack=…` must never become an option, and no push may use the `ext::` or another
//! `<helper>::` transport).
//!
//! Depends on: nothing of ours. Called by: `state`, `push`, `forge`, `gh`, `programs`.
//! Invariants: pure; anything unusual is refused (the caller says so in plain words), never repaired.

/// True when `name` can be used as a branch or remote name on a command line: what `git check-ref-format`
/// accepts for one ref component path, plus no leading `-`, no whitespace and at most 255 bytes.
#[must_use]
pub fn valid_ref_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 || name.starts_with('-') || name.starts_with('/') {
        return false;
    }
    if name.ends_with('/')
        || name.ends_with('.')
        || name.to_ascii_lowercase().ends_with(".lock")
        || name == "@"
    {
        return false;
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return false;
    }
    if name.chars().any(|c| {
        c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return false;
    }
    name.split('/')
        .all(|part| !part.is_empty() && !part.starts_with('.'))
}

/// A remote name: a valid ref name without `/` complications beyond what git allows.
#[must_use]
pub fn valid_remote_name(name: &str) -> bool {
    valid_ref_name(name)
}

/// What kind of address a push would use (after `insteadOf` / `pushInsteadOf`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// `https://`, `ssh://`, `git://`, `file://`, scp-like `host:path`, or a local path.
    Allowed,
    /// Anything else (`ext::`, `<helper>::`, `http://`, other schemes, option-like text).
    Refused,
}

/// Classifies a push address. Allowed: `https://`, `ssh://`, `git://`, `file://`, scp-like
/// `[user@]host:path`, and local paths. Refused: any `<transport>::` (remote helpers, `ext::`), plain
/// `http://` (credentials in clear), other schemes, empty, leading `-`, control characters.
#[must_use]
pub fn address_kind(url: &str) -> AddressKind {
    let url = url.trim();
    if url.is_empty()
        || url.starts_with('-')
        || url.chars().any(char::is_control)
        || url.contains("::")
    {
        return AddressKind::Refused;
    }
    if let Some((scheme, _)) = url.split_once("://") {
        return match scheme.to_ascii_lowercase().as_str() {
            "https" | "ssh" | "git" | "file" => AddressKind::Allowed,
            _ => AddressKind::Refused,
        };
    }
    // Windows drive paths (`C:\x`, `C:/x`) and plain paths are local repositories.
    let bytes = url.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return AddressKind::Allowed;
    }
    match url.find(':') {
        // scp-like: a host (no `/` before the colon).
        Some(colon) if !url[..colon].contains('/') => {
            if url[..colon].is_empty() {
                AddressKind::Refused
            } else {
                AddressKind::Allowed
            }
        }
        _ => AddressKind::Allowed,
    }
}
