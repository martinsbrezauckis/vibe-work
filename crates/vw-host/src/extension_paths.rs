//! Keeps only absolute folders in the extension path settings of the app-owned config before launch: Vibe Work
//! passes Vibe every extension folder as an absolute path. Depends on TOML and std; called by launch. Only
//! relative string entries and their commas change; errors warn and never stop launch.

use std::io::{self, Write};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

const KEYS: [&str; 2] = ["tool_paths", "agent_paths"];

pub(crate) fn prepare(config: &Path) {
    if let Err(reason) = update(config) {
        tracing::warn!(
            %reason,
            "extension paths: config.toml left unchanged; continuing launch for Vibe to report config errors"
        );
    }
}

fn update(config: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(config).map_err(|error| error.to_string())?;
    let (updated, removed) = without_relative_paths(&text).map_err(str::to_owned)?;
    if removed == [0, 0] {
        return Ok(());
    }
    replace_config(config, &text, &updated).map_err(|error| error.to_string())?;
    for (key, count) in KEYS.into_iter().zip(removed) {
        if count > 0 {
            tracing::warn!(
                key,
                count,
                "extension paths: relative entries are not used; only absolute folders are passed to Vibe"
            );
        }
    }
    Ok(())
}

fn replace_config(config: &Path, original: &str, updated: &str) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = config.with_extension(format!(
        "toml.vw-extensions-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let permissions = std::fs::metadata(config)?.permissions();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(updated.as_bytes())?;
        file.set_permissions(permissions)?;
        drop(file);
        // Keep an edit made while we were preparing the candidate. The launch can still proceed with it.
        if std::fs::read_to_string(config)? != original {
            return Err(io::Error::other(
                "config changed during extension path preparation",
            ));
        }
        std::fs::rename(&temp, config)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn without_relative_paths(text: &str) -> Result<(String, [usize; 2]), &'static str> {
    let parsed = toml::de::DeTable::parse(text).map_err(|_| "invalid Vibe config syntax")?;
    let mut expected = toml::Table::deserialize(toml::de::Deserializer::from(parsed.clone()))
        .map_err(|_| "invalid Vibe config values")?;
    let mut edits = Vec::new();
    let mut removed = [0, 0];
    for (slot, key) in KEYS.into_iter().enumerate() {
        let Some(value) = parsed.get_ref().get(key) else {
            continue;
        };
        let Some(paths) = value.get_ref().as_array() else {
            continue;
        };
        for (index, path) in paths.iter().enumerate() {
            if !path.get_ref().as_str().is_some_and(is_relative) {
                continue;
            }
            removed[slot] += 1;
            edits.push(path.span());
            let next = paths
                .get(index + 1)
                .map_or(value.span().end - 1, |next| next.span().start);
            if let Some(comma) = comma_between(text, path.span().end..next) {
                edits.push(comma);
            } else if index > 0
                && let Some(comma) =
                    comma_between(text, paths[index - 1].span().end..path.span().start)
            {
                edits.push(comma);
            }
        }
        if let Some(paths) = expected.get_mut(key).and_then(toml::Value::as_array_mut) {
            paths.retain(|path| !path.as_str().is_some_and(is_relative));
        }
    }
    // Commas shared by adjacent removed entries are deleted once. Work backwards so source spans stay valid.
    edits.sort_unstable_by_key(|range| range.start);
    edits.dedup();
    let mut updated = text.to_owned();
    for range in edits.into_iter().rev() {
        updated.replace_range(range, "");
    }
    let verified: toml::Table =
        toml::from_str(&updated).map_err(|_| "extension path edit could not be parsed")?;
    // Compare canonical data without persisting it: TOML's NaN is unequal to itself in Value::PartialEq.
    let expected = toml::to_string(&expected).map_err(|_| "config values could not be verified")?;
    let verified = toml::to_string(&verified).map_err(|_| "config values could not be verified")?;
    if verified != expected {
        return Err("extension path edit would change other config values");
    }
    Ok((updated, removed))
}

fn is_relative(path: &str) -> bool {
    !Path::new(path).is_absolute()
}

/// Gaps between parsed array values contain only separators, whitespace and comments. Ignore comment commas.
fn comma_between(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let gap = text.get(range.clone())?;
    let mut comment = false;
    for (offset, ch) in gap.char_indices() {
        match ch {
            '#' => comment = true,
            '\n' => comment = false,
            ',' if !comment => return Some(range.start + offset..range.start + offset + 1),
            _ => {}
        }
    }
    None
}
