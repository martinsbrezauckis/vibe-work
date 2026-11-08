//! Removes app-managed proxy credentials from legacy files after vault unlock, before Vibe can spawn.
//! Uses the existing vault and atomic private-file writer; only proxy assignments and our reference comments change.

use crate::{
    KeyError,
    proxy_credentials::{self, PROXIES},
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vw_mistral::network::{NetworkSettings, has_sign_in, with_loopback};
use vw_vault::Vault;

const PREFIX: &str = "# Vibe Work proxy vault ";
const ROUTE_PREFIX: &str = "# Vibe Work proxy route ";

fn error() -> KeyError {
    KeyError::Settings(
        "Proxy sign-in migration could not finish. Unlock the vault and restart Vibe Work.".into(),
    )
}

fn text(path: &Path) -> Result<String, KeyError> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.len() <= 1024 * 1024 => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        _ => Err(error()),
    }
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let line = line
        .strip_prefix("export")
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map_or(line, str::trim_start);
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let key = if let Some(key) = key.strip_prefix('\'') {
        key.strip_suffix('\'')?
    } else {
        key
    };
    let key = PROXIES
        .into_iter()
        .chain(["NO_PROXY"])
        .find(|owned| owned.eq_ignore_ascii_case(key))
        .unwrap_or(key);
    Some((key, value))
}

fn assignment(line: &str) -> Option<(&str, String)> {
    let (key, value) = key_value(line)?;
    let value = value.trim();
    let value = if value.starts_with(['\'', '"']) {
        quoted(value)?
    } else {
        let comment = value.char_indices().find_map(|(index, ch)| {
            (ch == '#' && value[..index].ends_with(char::is_whitespace)).then_some(index)
        });
        value[..comment.unwrap_or(value.len())].trim().to_owned()
    };
    Some((key, value))
}

fn quoted(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        if ch == quote {
            let rest = chars.as_str().trim();
            return (rest.is_empty() || rest.starts_with('#')).then_some(result);
        }
        if ch == '\\' {
            let next = chars.next()?;
            let decoded = match next {
                '\\' | '\'' => Some(next),
                '"' if quote == '"' => Some('"'),
                'a' if quote == '"' => Some('\x07'),
                'b' if quote == '"' => Some('\x08'),
                'f' if quote == '"' => Some('\x0c'),
                'n' if quote == '"' => Some('\n'),
                'r' if quote == '"' => Some('\r'),
                't' if quote == '"' => Some('\t'),
                'v' if quote == '"' => Some('\x0b'),
                _ => None,
            };
            if decoded.is_none() {
                result.push('\\');
            }
            result.push(decoded.unwrap_or(next));
        } else {
            result.push(ch);
        }
    }
    None
}

/// Ambiguous owned assignments must stop launch, not leave a possibly readable credential in the file.
fn values(text: &str) -> Result<BTreeMap<String, String>, KeyError> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let parsed = assignment(line);
        if parsed.is_none()
            && key_value(line).is_some_and(|(key, _)| vw_mistral::network::KEYS.contains(&key))
        {
            return Err(error());
        }
        if let Some((key, value)) = parsed {
            // Conflicting aliases or duplicate assignments could hide a sign-in behind a public value.
            if (PROXIES.contains(&key) || key == "NO_PROXY")
                && values.get(key).is_some_and(|old| old != &value)
            {
                return Err(error());
            }
            values.insert(key.to_owned(), value);
        }
    }
    Ok(values)
}

pub(crate) fn read(path: &Path) -> Result<NetworkSettings, KeyError> {
    let text = text(path)?;
    let values = values(&text)?;
    let mut settings = NetworkSettings::from_lookup(|key| values.get(key).cloned());
    // Restore the durable route only when its assignment still matches the previous run's capability.
    // A person-edited proxy remains authoritative. This also covers SOCKS routes without a vault sign-in.
    for line in text
        .lines()
        .filter_map(|line| line.strip_prefix(ROUTE_PREFIX))
    {
        let (key, local, endpoint): (String, String, String) =
            serde_json::from_str(line).map_err(|_| error())?;
        if !PROXIES.contains(&key.as_str()) || has_sign_in(&endpoint) {
            return Err(error());
        }
        if values.get(&key) == Some(&local) {
            settings = settings.with(&key, Some(endpoint));
        }
    }
    for line in text.lines().filter_map(|line| line.strip_prefix(PREFIX)) {
        let Ok((key, id, endpoint)) = serde_json::from_str::<(String, String, String)>(line) else {
            return Err(error());
        };
        if !PROXIES.contains(&key.as_str()) {
            return Err(error());
        }
        if settings.get(&key).is_some_and(|v| has_sign_in(&v)) {
            continue;
        }
        settings = settings.with(&key, Some(endpoint));
        settings.proxy_credentials.insert(key, id);
    }
    Ok(settings)
}

/// Credentials discovered in Vibe's file are only Vibe's settings, never core-client routing authority.
pub(crate) fn migrate(vault: &Vault, path: &Path) -> Result<NetworkSettings, KeyError> {
    let settings = read(path)?;
    proxy_credentials::store(vault, &settings)
}

/// Rewrite only owned assignments. Unknown lines remain byte-for-byte; the new file and directory are synced.
pub(crate) fn write(
    path: &Path,
    settings: &NetworkSettings,
    forwarder: &vw_netguard::NetGuard,
) -> Result<(), KeyError> {
    let old = text(path)?;
    values(&old)?;
    let mut lines: Vec<String> = old
        .lines()
        .filter(|line| {
            if line.starts_with(PREFIX) || line.starts_with(ROUTE_PREFIX) {
                return false;
            }
            !assignment(line).is_some_and(|(key, _)| PROXIES.contains(&key) || key == "NO_PROXY")
        })
        .map(str::to_owned)
        .collect();
    let child = crate::proxy_forwarder::public(settings, forwarder);
    for key in PROXIES {
        if let Some(value) = child.get(key) {
            if value == forwarder.proxy_url() {
                let endpoint = settings.get(key).ok_or_else(error)?;
                if has_sign_in(&endpoint) {
                    return Err(error());
                }
                let route = serde_json::to_string(&(key, &value, endpoint)).map_err(|_| error())?;
                lines.push(format!("{ROUTE_PREFIX}{route}"));
            } else if has_sign_in(&value) {
                return Err(error());
            }
            lines.push(format!(
                "{key}='{}'",
                value.replace('\\', "\\\\").replace('\'', "\\'")
            ));
        }
        if let (Some(id), Some(endpoint)) = (settings.proxy_credentials.get(key), settings.get(key))
        {
            let reference = serde_json::to_string(&(key, id, endpoint)).map_err(|_| error())?;
            lines.push(format!("{PREFIX}{reference}"));
        }
    }
    if child.has_proxy() || child.no_proxy.is_some() {
        lines.push(format!(
            "NO_PROXY='{}'",
            if child.has_proxy() {
                with_loopback(child.no_proxy.as_deref().unwrap_or_default())
            } else {
                child.no_proxy.clone().unwrap_or_default()
            }
        ));
    }
    if old.is_empty() && lines.is_empty() {
        return cleanup(path);
    }
    let new = format!("{}\n", lines.join("\n"));
    if old != new {
        replace(path, new.as_bytes())?;
    }
    cleanup(path)
}

pub(crate) fn replace(path: &Path, bytes: &[u8]) -> Result<(), KeyError> {
    let parent = path.parent().ok_or_else(error)?;
    vw_platform::ensure_private_dir(parent).map_err(|_| error())?;
    vw_platform::replace_private_durable(path, bytes).map_err(|_| error())?;
    sync_parent(path)
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "Unix directory sync can fail; Windows uses a write-through rename"
)]
fn sync_parent(path: &Path) -> Result<(), KeyError> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    std::fs::File::open(path.parent().ok_or_else(error)?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| error())?;
    Ok(())
}

/// Discovery and removal share the exact writer-owned namespace, including python-dotenv's sibling temps.
fn remnant_paths(path: &Path) -> Result<Vec<PathBuf>, KeyError> {
    let Some(parent) = path.parent().filter(|p| p.exists()) else {
        return Ok(Vec::new());
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(error)?;
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(parent).map_err(|_| error())? {
        let entry = entry.map_err(|_| error())?;
        let candidate = entry.file_name().to_string_lossy().into_owned();
        let owned = candidate.starts_with(&format!("{name}.new-"))
            || candidate == format!("{name}.bak")
            || candidate == format!("{name}.tmp")
            || (name == "settings.json" && candidate == "settings.tmp")
            || (name == ".env" && candidate.starts_with(".tmp_"));
        if owned && entry.file_type().map_err(|_| error())?.is_file() {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

/// Remnants may hold older, different sign-ins. Quarantine each without granting it routing authority.
pub(crate) fn remnant_sign_ins(
    core: Option<&Path>,
    dotenv: &Path,
) -> Result<Vec<NetworkSettings>, KeyError> {
    let mut found = Vec::new();
    for main in std::iter::once(dotenv).chain(core) {
        for path in remnant_paths(main)? {
            let raw = text(&path)?;
            if raw.trim().is_empty() {
                continue;
            }
            // Also preserve old copies containing only the URL, rather than a complete record.
            let settings = if url::Url::parse(raw.trim()).is_ok() && has_sign_in(raw.trim()) {
                NetworkSettings::default().with("ALL_PROXY", Some(raw.trim().to_owned()))
            } else if main == dotenv {
                read(&path)?
            } else {
                untrusted(&path)?
            };
            if PROXIES
                .iter()
                .any(|key| settings.get(key).is_some_and(|v| has_sign_in(&v)))
            {
                found.push(settings);
            }
        }
    }
    Ok(found)
}

/// Remove only known writer remnants after their sign-ins have been stored; sync even for an empty main file.
pub(crate) fn cleanup(path: &Path) -> Result<(), KeyError> {
    for remnant in remnant_paths(path)? {
        std::fs::remove_file(remnant).map_err(|_| error())?;
    }
    if path.parent().is_some_and(Path::exists) {
        sync_parent(path)?;
    }
    Ok(())
}

/// Detect a legacy sign-in even before the MAC key is available; never authorizes these values for a client.
pub(crate) fn legacy(path: &Path) -> Result<bool, KeyError> {
    let text = text(path)?;
    if text.is_empty() {
        return Ok(false);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(false);
    };
    let value = if let Some(payload) = value["settings"].as_str() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            return Ok(false);
        };
        value
    } else {
        value
    };
    let values = value.get("values").unwrap_or(&value);
    Ok(PROXIES
        .iter()
        .any(|key| values[*key].as_str().is_some_and(has_sign_in)))
}

/// Extract legacy values solely to quarantine their sign-ins in the vault when authentication failed.
/// The result must never configure a core client.
pub(crate) fn untrusted(path: &Path) -> Result<NetworkSettings, KeyError> {
    let value: serde_json::Value = serde_json::from_str(&text(path)?).map_err(|_| error())?;
    let value = if let Some(payload) = value["settings"].as_str() {
        serde_json::from_str::<serde_json::Value>(payload).map_err(|_| error())?
    } else {
        value
    };
    let values = value.get("values").unwrap_or(&value);
    Ok(NetworkSettings::from_lookup(|key| {
        PROXIES
            .contains(&key)
            .then(|| values[key].as_str().map(str::to_owned))
            .flatten()
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn mixed_case_proxy_names_cannot_bypass_migration() {
        for key in ["Http_Proxy", "hTtPs_pRoXy", "All_Proxy", "No_Proxy"] {
            let values = super::values(&format!(
                "{key}='http://alice:synthetic-secret@proxy.invalid:8080'"
            ))
            .unwrap();
            assert!(
                values.contains_key(&key.to_ascii_uppercase()),
                "{key} was not discovered"
            );
            assert!(super::values(&format!("{key}='unfinished")).is_err());
        }
    }

    #[test]
    fn lowercase_proxy_assignments_are_owned_and_ambiguous_forms_stop_launch() {
        for key in ["http_proxy", "https_proxy", "all_proxy", "no_proxy"] {
            assert!(super::values(&format!("{key}='unfinished")).is_err());
            assert!(super::values(&format!("{key}=\"value\" unexpected")).is_err());
            let values = super::values(&format!("export '{key}'='value' # comment")).unwrap();
            assert_eq!(
                values.get(&key.to_ascii_uppercase()).map(String::as_str),
                Some("value")
            );
            assert!(
                super::values(&format!("{key}=one\n{}=two", key.to_ascii_uppercase())).is_err()
            );
        }
    }

    #[test]
    fn quoted_proxy_with_a_comment_is_found_for_migration() {
        assert_eq!(
            super::assignment("export HTTPS_PROXY='http://alice:s3cret@proxy:8080' # company"),
            Some(("HTTPS_PROXY", "http://alice:s3cret@proxy:8080".into()))
        );
    }

    #[test]
    fn ambiguous_owned_values_stop_migration() {
        assert!(super::values("HTTPS_PROXY='http://alice:s3cret@proxy:8080").is_err());
        assert!(
            super::values("HTTPS_PROXY=\"http://alice:s3cret@proxy:8080\" unexpected").is_err()
        );
        assert!(super::values("UNRELATED='unfinished").is_ok());
    }

    #[test]
    fn dotenv_quoting_and_escapes_are_preserved() {
        assert_eq!(
            super::assignment(
                "export\t'HTTPS_PROXY'=\"http://alice:p\\\"ass@proxy:8080\"\t# comment"
            ),
            Some(("HTTPS_PROXY", "http://alice:p\"ass@proxy:8080".into()))
        );
        assert_eq!(
            super::assignment("HTTPS_PROXY=http://proxy:8080\t# comment"),
            Some(("HTTPS_PROXY", "http://proxy:8080".into()))
        );
    }
}
