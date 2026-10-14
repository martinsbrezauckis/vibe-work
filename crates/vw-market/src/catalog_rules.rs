//! The per-kind rules of [`crate::catalog::Catalog::validate`]: hashes in full form, relative paths without `..`,
//! addresses through [`crate::catalog::check_url`], the kind's own block present and the others absent.
//!
//! Depends on: `catalog` (types). Called by: `catalog::Catalog::validate` only.
//! Invariant: every function returns the broken rule as plain text (the caller adds the entry id); none echoes an
//! address.

use crate::catalog::{
    Entry, FileSpec, Kind, McpServerSpec, PluginSpec, REQUIRED_TARGETS, Rules, check_url,
};

type Rule = Result<(), String>;

/// Checks the block that belongs to `entry.kind`.
pub(crate) fn check_kind(entry: &Entry, rules: Rules) -> Rule {
    let blocks = (
        entry.pack.is_some(),
        entry.server.is_some(),
        entry.plugin.is_some(),
    );
    match (entry.kind, blocks) {
        (Kind::Pack, (true, false, false)) => check_pack(entry, rules),
        (Kind::Server, (false, true, false)) => {
            let server = entry.server.as_ref().ok_or("missing server")?;
            check_url(&server.url, rules.allow_loopback_http).map_err(str::to_owned)?;
            plain(&server.checked)
        }
        (Kind::Plugin, (false, false, true)) => {
            check_plugin(entry.plugin.as_ref().ok_or("missing plugin")?, rules)
        }
        _ => Err("exactly the block of its kind must be present".to_owned()),
    }
}

fn check_pack(entry: &Entry, rules: Rules) -> Rule {
    let pack = entry.pack.as_ref().ok_or("missing pack")?;
    plain(&pack.version)?;
    if pack.files.is_empty() && pack.mcp_servers.is_empty() {
        return Err("a pack needs files or servers".to_owned());
    }
    if !pack.files.is_empty() || pack.licence_file.is_some() {
        let source = pack.source.as_ref().ok_or("vendor files need a source")?;
        if !is_repo(&source.repo) || !is_hex(&source.commit, 40) {
            return Err("source must be owner/name at a full 40-character commit".to_owned());
        }
        if let Some(url) = &source.archive_url {
            check_url(url, rules.allow_loopback_http).map_err(str::to_owned)?;
        }
        if source.max_bytes == 0 || source.max_bytes > crate::catalog::MAX_DOWNLOAD_BYTES {
            return Err("source maxBytes out of range".to_owned());
        }
        if pack.licence_file.is_none() {
            return Err("vendor files need the vendor's licence file".to_owned());
        }
    }
    let mut targets = std::collections::BTreeSet::new();
    for file in pack.files.iter().chain(pack.licence_file.iter()) {
        check_file(file)?;
    }
    for file in &pack.files {
        if !file.to.starts_with("skills/") || !targets.insert(file.to.as_str()) {
            return Err("pack files go under skills/, each once".to_owned());
        }
    }
    for edit in &pack.edits {
        if !targets.contains(edit.file.as_str())
            || edit.find.is_empty()
            || !is_hex(&edit.result_sha256, 64)
        {
            return Err("an edit must name a pack file, a text and a result hash".to_owned());
        }
    }
    for (name, server) in &pack.mcp_servers {
        check_server_name(name)?;
        match server {
            McpServerSpec::StreamableHttp { url } => {
                check_url(url, rules.allow_loopback_http).map_err(str::to_owned)?;
            }
            McpServerSpec::Stdio { .. } => {
                return Err("packs may only list remote servers".to_owned());
            }
        }
    }
    Ok(())
}

fn check_plugin(plugin: &PluginSpec, rules: Rules) -> Rule {
    plain(&plugin.version)?;
    plain(&plugin.consent)?;
    if rules.require_all_targets {
        for target in REQUIRED_TARGETS {
            if !plugin.runtime.targets.contains_key(*target) {
                return Err(format!(
                    "runtime target {target} missing (same features on every OS)"
                ));
            }
        }
    }
    for target in plugin.runtime.targets.values() {
        check_url(&target.url, rules.allow_loopback_http).map_err(str::to_owned)?;
        if !is_hex(&target.sha256, 64)
            || target.bytes == 0
            || target.bytes > crate::catalog::MAX_DOWNLOAD_BYTES
        {
            return Err("runtime target needs a sha256 and a size".to_owned());
        }
        check_path(&target.executable)?;
        if !target.executable.starts_with("runtime/")
            || !target.files.iter().any(|f| f.to == target.executable)
        {
            return Err(
                "the runtime executable must be one of the runtime files under runtime/".to_owned(),
            );
        }
        for member in &target.files {
            check_path(&member.from)?;
            check_path(&member.to)?;
            if !member.to.starts_with("runtime/") {
                return Err("runtime files go under runtime/".to_owned());
            }
        }
    }
    for package in &plugin.npm {
        check_url(&package.url, rules.allow_loopback_http).map_err(str::to_owned)?;
        if !is_npm_name(&package.name) || plain(&package.version).is_err() {
            return Err("npm package needs a plain name and version".to_owned());
        }
        if crate::digest::parse_sri_sha512(&package.integrity).is_none() {
            return Err("npm integrity must be sha512-<base64>".to_owned());
        }
        if package.max_bytes == 0 || package.max_bytes > crate::catalog::MAX_DOWNLOAD_BYTES {
            return Err("npm maxBytes out of range".to_owned());
        }
    }
    if let Some(step) = &plugin.post_install {
        plain(&step.what)?;
        if step.args.is_empty() || step.timeout_secs == 0 || step.timeout_secs > 3600 {
            return Err("post-install needs arguments and a timeout up to an hour".to_owned());
        }
    }
    if plugin.mcp_servers.is_empty() {
        return Err("a plugin needs at least one server".to_owned());
    }
    for (name, server) in &plugin.mcp_servers {
        check_server_name(name)?;
        if let McpServerSpec::StreamableHttp { url } = server {
            check_url(url, rules.allow_loopback_http).map_err(str::to_owned)?;
        }
    }
    for (to, bundled) in &plugin.bundled {
        check_path(to)?;
        if crate::assemble::bundled(bundled).is_none() {
            return Err(format!("bundled file {bundled} is not part of this build"));
        }
    }
    Ok(())
}

fn check_file(file: &FileSpec) -> Rule {
    check_path(&file.from)?;
    check_path(&file.to)?;
    if is_hex(&file.sha256, 64) {
        Ok(())
    } else {
        Err("file hashes must be 64 lowercase hex characters".to_owned())
    }
}

/// A relative path with `/` separators, no empty, `.` or `..` parts, no drive or backslash, at most 512 bytes.
pub(crate) fn check_path(path: &str) -> Rule {
    let ok = !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if ok {
        Ok(())
    } else {
        Err("paths must be relative, with no empty, . or .. parts".to_owned())
    }
}

fn check_server_name(name: &str) -> Rule {
    if crate::catalog::is_plugin_name(name) {
        Ok(())
    } else {
        Err("server names use a-z, 0-9 and dashes".to_owned())
    }
}

fn plain(text: &str) -> Rule {
    if text.trim().is_empty() || text.chars().any(char::is_control) {
        Err("texts must be non-empty plain text".to_owned())
    } else {
        Ok(())
    }
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                && p != "."
                && p != ".."
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

fn is_npm_name(name: &str) -> bool {
    let body = name
        .strip_prefix('@')
        .map_or(Some(name), |scoped| scoped.split_once('/').map(|(_, n)| n));
    body.is_some_and(|b| !b.is_empty())
        && name.len() <= 214
        && name.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'-' | b'_' | b'.' | b'@' | b'/')
        })
        && !name.contains("..")
}
