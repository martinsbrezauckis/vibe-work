//! Pins unsupported experiments off in the app-owned Vibe config and migrates retired keys.
//! Depends on TOML parsing/editing; called by both launch paths before Vibe starts.
//! Registry skills default on because skill offers need them on this Vibe version; recheck at every Vibe pin bump.
//! Migrates the former app-written registry false once, then preserves personal choices. A separate app-owned
//! marker survives Vibe config rewrites. Preserves access mode, comments and unrelated values. Invalid TOML is left for
//! Vibe to diagnose. The parsed result must match the intended migration before writing.

use std::path::Path;

use toml::{Table, Value};
use toml_edit::{DocumentMut, value};

/// The keys pinned off (top level of Vibe's config).
pub const KEYS: [&str; 8] = vw_supervisor::EXPERIMENT_CONFIG_KEYS;

/// Registry skills have their own default; all unsupported experiments remain unavailable.
const NOTE: &str = "# Vibe Work: unsupported experimental features stay off.";
const PREVIOUS_NOTE: &str = "# Vibe Work: Mistral's experimental features stay off.";
const OLD_NOTE: &str =
    "# Vibe Work: Mistral's experiments do not switch these on (set one to true to opt in).";
const OLD_COW: &str = "experimental_worktree_copy_on_write";
const COW: &str = "worktree_copy_on_write";
const REGISTRY: &str = "experimental_enable_registry_skills";
// App-owned state in the managed home, outside the config Vibe rewrites; removed with that home.
const REGISTRY_MIGRATION: &str = ".vw-registry-skills-default-v1";

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentPins {
    /// The keys added as `false`.
    pub added: Vec<&'static str>,
    /// Existing experimental keys reset to `false`.
    pub disabled: Vec<&'static str>,
    /// Why the file was left unchanged, when it was not valid TOML or the edit could not be verified.
    pub skipped: Option<String>,
}

impl ExperimentPins {
    fn skipped(why: &str) -> Self {
        Self {
            added: Vec::new(),
            disabled: Vec::new(),
            skipped: Some(why.to_owned()),
        }
    }
}

/// Returns the migrated config and pin outcome, or `None` if no values need changing. Pure.
#[must_use]
pub fn with_pins_off(existing: &str) -> (Option<String>, ExperimentPins) {
    edited(existing, false)
}

fn edited(existing: &str, migrate_registry: bool) -> (Option<String>, ExperimentPins) {
    let Ok(mut expected) = existing.parse::<Table>() else {
        return (
            None,
            ExperimentPins::skipped("config.toml is not valid TOML; Vibe reports the error itself"),
        );
    };
    let Ok(mut doc) = existing.parse::<DocumentMut>() else {
        return (
            None,
            ExperimentPins::skipped("config.toml could not be edited"),
        );
    };
    let mut outcome = ExperimentPins {
        added: Vec::new(),
        disabled: Vec::new(),
        skipped: None,
    };
    doc.remove("smart_approve_default");
    expected.remove("smart_approve_default");
    if let Some(old) = doc.remove(OLD_COW)
        && !doc.contains_key(COW)
    {
        doc[COW] = old;
    }
    if let Some(old) = expected.remove(OLD_COW) {
        expected.entry(COW).or_insert(old);
    }
    // Preserve the previous stable default when a remote default-agent experiment is active.
    if !doc.contains_key("default_agent") {
        doc["default_agent"] = value("accept-edits");
        expected.insert("default_agent".into(), Value::String("accept-edits".into()));
    }
    if !doc.contains_key(REGISTRY)
        || (migrate_registry && expected.get(REGISTRY).and_then(Value::as_bool) == Some(false))
    {
        let decor = doc
            .get(REGISTRY)
            .and_then(toml_edit::Item::as_value)
            .map(|v| v.decor().clone());
        doc[REGISTRY] = value(true);
        if let (Some(decor), Some(new)) = (decor, doc[REGISTRY].as_value_mut()) {
            *new.decor_mut() = decor;
        }
        expected.insert(REGISTRY.into(), Value::Boolean(true));
    }
    for key in KEYS {
        match expected.get(key) {
            None => outcome.added.push(key),
            Some(Value::Boolean(false)) => continue,
            Some(_) => outcome.disabled.push(key),
        }
        let decor = doc
            .get(key)
            .and_then(toml_edit::Item::as_value)
            .map(|v| v.decor().clone());
        doc[key] = value(false);
        if let (Some(decor), Some(new)) = (decor, doc[key].as_value_mut()) {
            *new.decor_mut() = decor;
        }
        expected.insert(key.into(), Value::Boolean(false));
    }
    let values_changed = doc.to_string() != existing;
    let mut out = doc
        .to_string()
        .replace(OLD_NOTE, NOTE)
        .replace(PREVIOUS_NOTE, NOTE);
    if !out.contains(NOTE) {
        out = if values_changed {
            format!("{NOTE}\n{out}")
        } else {
            crate::config_comments::insert_missing(&out, None, REGISTRY, &[NOTE])
                .unwrap_or_else(|| format!("{NOTE}\n{out}"))
        };
    }
    if existing.contains("\r\n") {
        out = out.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    if !out.parse::<Table>().is_ok_and(|after| after == expected) {
        return (
            None,
            ExperimentPins::skipped("the edit could not be verified"),
        );
    }
    if out == existing {
        (None, outcome)
    } else {
        (Some(out), outcome)
    }
}

/// Migrates `vibe_home/config.toml` and pins experiments off. A missing file is created.
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<ExperimentPins, String> {
    let path = vibe_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let marker = vibe_home.join(REGISTRY_MIGRATION);
    let migrated = marker
        .try_exists()
        .map_err(|e| format!("{}: {e}", marker.display()))?;
    let (updated, outcome) = edited(&existing, !migrated);
    if let Some(updated) = updated {
        std::fs::create_dir_all(vibe_home).map_err(|e| format!("{}: {e}", vibe_home.display()))?;
        let tmp = vibe_home.join("config.toml.vw-tmp");
        std::fs::write(&tmp, &updated).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })?;
    } else if outcome.skipped.is_none() {
        crate::config_comments::restore(&path, &existing, None, REGISTRY, &[NOTE])?;
    }
    // Record only after a verified config write (or no-op); invalid TOML must not consume the step.
    if !migrated && outcome.skipped.is_none() {
        std::fs::write(&marker, b"1\n").map_err(|e| format!("{}: {e}", marker.display()))?;
    }
    Ok(outcome)
}
