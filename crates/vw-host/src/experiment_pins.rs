//! Pins unsupported experiments off in the app-owned Vibe config and migrates retired keys.
//! Depends on TOML parsing/editing; written by [`crate::startup_config`] in both launch paths before Vibe starts.
//! Registry skills and the model catalog default on. Separate app-owned markers record each first default so a later
//! personal choice survives Vibe config rewrites. Preserves access mode, comments and unrelated values. Invalid TOML
//! is left for Vibe to diagnose. The parsed result must match the intended migration before writing.

use std::path::{Path, PathBuf};

use crate::startup_config::{Draft, Staged, Written};

use toml::{Table, Value};
use toml_edit::{DocumentMut, value};

/// The keys pinned off (top level of Vibe's config).
pub const KEYS: [&str; 5] = vw_supervisor::EXPERIMENT_CONFIG_KEYS;

/// Stable features disabled until the app offers them through its own controls.
pub const STABLE_KEYS: [&str; 1] = vw_supervisor::STABLE_CONFIG_KEYS;

/// Registry skills have their own default; all unsupported experiments remain unavailable.
const NOTE: &str = "# Vibe Work: unsupported experimental features stay off.";
const PREVIOUS_NOTE: &str = "# Vibe Work: Mistral's experimental features stay off.";
const OLD_NOTE: &str =
    "# Vibe Work: Mistral's experiments do not switch these on (set one to true to opt in).";
const OLD_COW: &str = "experimental_worktree_copy_on_write";
const COW: &str = "worktree_copy_on_write";
const REGISTRY: &str = "experimental_enable_registry_skills";
const MODEL_CATALOG: &str = "experimental_enable_model_catalog";
// App-owned state in the managed home, outside the config Vibe rewrites; removed with that home.
const REGISTRY_MIGRATION: &str = ".vw-registry-skills-default-v1";
const CATALOG_MIGRATION: &str = ".vw-model-catalog-default-v1";

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
    edited(existing, false, true)
}

fn enable(doc: &mut DocumentMut, expected: &mut Table, key: &str) {
    let decor = doc
        .get(key)
        .and_then(toml_edit::Item::as_value)
        .map(|v| v.decor().clone());
    doc[key] = value(true);
    if let (Some(decor), Some(new)) = (decor, doc[key].as_value_mut()) {
        *new.decor_mut() = decor;
    }
    expected.insert(key.into(), Value::Boolean(true));
}

fn edited(
    existing: &str,
    migrate_registry: bool,
    migrate_catalog: bool,
) -> (Option<String>, ExperimentPins) {
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
    // The stable replacement defaults on. Retired keys also fail strict config validation.
    // Carry the old monitoring line's comment to the replacement, then pin its value below.
    if let Some(old) = doc.remove("experimental_enable_pr_monitoring")
        && !doc.contains_key("enable_pr_monitoring")
    {
        doc["enable_pr_monitoring"] = old;
    }
    if let Some(old) = expected.remove("experimental_enable_pr_monitoring") {
        expected.entry("enable_pr_monitoring").or_insert(old);
    }
    doc.remove("experimental_enable_pr_discovery");
    expected.remove("experimental_enable_pr_discovery");
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
        enable(&mut doc, &mut expected, REGISTRY);
    }
    // On the first launch with this default, replace a missing or disabled catalog choice.
    // The marker makes later changes to this key the person's own choice.
    if migrate_catalog
        && matches!(
            expected.get(MODEL_CATALOG),
            None | Some(Value::Boolean(false))
        )
    {
        enable(&mut doc, &mut expected, MODEL_CATALOG);
    }
    for key in KEYS.into_iter().chain(STABLE_KEYS) {
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

/// The experiment step staged in a [`Draft`]: its outcome and the registry-migration marker state.
pub(crate) struct Stage {
    marker: PathBuf,
    catalog_marker: PathBuf,
    /// Whether the registry migration ran before, or why the marker could not be checked.
    migrated: Result<bool, String>,
    catalog_migrated: Result<bool, String>,
    pins: Staged<ExperimentPins>,
}

impl Stage {
    /// Whether this is the first completed-attempt window for the catalog default and model cleanup.
    pub(crate) fn catalog_first_launch(&self) -> bool {
        self.catalog_migrated
            .as_ref()
            .is_ok_and(|recorded| !*recorded)
    }
}

/// Runs this step's edit over `draft` (the registry migration only while its marker is absent).
pub(crate) fn stage(draft: &mut Draft, vibe_home: &Path) -> Stage {
    let marker = vibe_home.join(REGISTRY_MIGRATION);
    let catalog_marker = vibe_home.join(CATALOG_MIGRATION);
    let migrated = marker
        .try_exists()
        .map_err(|e| format!("{}: {e}", marker.display()));
    let catalog_migrated = catalog_marker
        .try_exists()
        .map_err(|e| format!("{}: {e}", catalog_marker.display()));
    let pins = match (&migrated, &catalog_migrated) {
        (Ok(migrated), Ok(catalog_migrated)) => draft.apply(
            |text, _| {
                let (updated, outcome) = edited(text, !*migrated, !*catalog_migrated);
                if updated.is_none() && outcome.skipped.is_none() {
                    let notes = crate::config_comments::restored(text, None, REGISTRY, &[NOTE]);
                    return (notes, outcome);
                }
                (updated, outcome)
            },
            |why| ExperimentPins::skipped(&why),
        ),
        _ => Staged {
            outcome: ExperimentPins::skipped("a default migration marker could not be checked"),
            changed: false,
        },
    };
    Stage {
        marker,
        catalog_marker,
        migrated,
        catalog_migrated,
        pins,
    }
}

/// The step's outcome after the combined write. Records the registry migration only when the file holds every
/// startup edit (written, or nothing to write) and this step was not skipped: a refused, failed or invalid config
/// must not consume the migration.
///
/// # Errors
/// The marker could not be checked or written, or this step's edit was part of a failed write.
pub(crate) fn finish(stage: Stage, written: &Written) -> Result<ExperimentPins, String> {
    let migrated = stage.migrated?;
    let catalog_migrated = stage.catalog_migrated?;
    let pins = written.settle(stage.pins, |why| ExperimentPins::skipped(&why))?;
    if !migrated && pins.skipped.is_none() && written.landed() {
        std::fs::write(&stage.marker, b"1\n")
            .map_err(|e| format!("{}: {e}", stage.marker.display()))?;
    }
    if !catalog_migrated && pins.skipped.is_none() && written.landed() {
        std::fs::write(&stage.catalog_marker, b"1\n")
            .map_err(|e| format!("{}: {e}", stage.catalog_marker.display()))?;
    }
    Ok(pins)
}

/// Migrates `vibe_home/config.toml` and pins experiments off, through the guarded startup write
/// ([`crate::startup_config`]). A missing file is created.
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<ExperimentPins, String> {
    let mut draft = Draft::open(vibe_home)
        .map_err(|e| format!("{}: {e}", vibe_home.join("config.toml").display()))?;
    let stage = stage(&mut draft, vibe_home);
    let written = draft.commit();
    finish(stage, &written)
}
