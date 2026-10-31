//! Mistral's remote experiments cannot switch on Vibe features that would work outside Vibe Work's guards: Vibe Work's
//! own `VIBE_HOME/config.toml` gets `false` for each key in [`KEYS`] that it does not set.
//!
//! Why: Vibe maps Mistral's `GrowthBook` experiments onto config keys (`core/config/layers/growthbook.py`
//! `GROWTHBOOK_CONFIG_MAPPINGS`), in a layer below the user file (`core/config/default_orchestrator.py`: Default <
//! `ModelCatalog` < `Growthbook` < user < project < ...), so the user file's value wins. Without it, an experiment turns on:
//! - `experimental_enable_pr_monitoring`: Vibe runs `gh pr list` / `glab mr list` in the session's repository and
//!   reads `gh pr create` results, with the person's own gh/glab login, outside the agent's GitHub rule
//!   (`app_server/_unified_harness_backend_adapter.py` `schedule_pr_discovery`, `schedule_artifact_sync`);
//! - `experimental_enable_registry_skills`: at session start Vibe downloads the registry skills a repository's
//!   committed `.vibe/skills.toml` pins and gives them to the agent, with no install card
//!   (`core/skills/registry/_service.py` `refresh_registry_skills`, `core/skills/manager.py`);
//! - `smart_approve_default`: new chats start in `smart-approve` (a classifier runs the calls it judges safe without a
//!   card) even when the person's own `default_agent` is stricter (`core/agents/manager.py` `resolve_default_agent`
//!   puts it above `default_agent`). Vibe Work starts a new chat without naming a mode, so Vibe's default applies.
//!
//! The experiments that are not pinned (system prompt, model routing and extra models, managed shell tools, the
//! smart-approve mode being offered) change no guard: they only change text or the model choice, or apply only to
//! Vibe's legacy tool surface, or offer a mode the person picks.
//!
//! The person's own value is kept: a config that already sets a key (their hand edit, or `false` from an earlier
//! start) is left as it is, and a value other than `false` is logged as kept. The window and the agent cannot write
//! these keys (the gate's config rules refuse `experimental_*`; Vibe hides the `smart_approve_*` settings; the folder
//! guard covers `VIBE_HOME`). Vibe's own rewrites of the file keep the keys (its user layer is written back whole).
//!
//! Depends on: `toml` (parsing only; the file is edited as text so the user's comments and layout stay).
//! Called by: `launch` (product and development launches, before the server starts), tests.
//! Invariants: only Vibe Work's own `VIBE_HOME` (never `~/.vibe`); a file that is not valid TOML is left byte-identical;
//! the edit is verified (the new file parses and equals the old data plus the added keys, each `false`) before a
//! temp-file rename.

use std::path::Path;

use toml::{Table, Value};

/// The keys pinned off (top level of Vibe's config).
pub const KEYS: [&str; 3] = [
    "experimental_enable_pr_monitoring",
    "experimental_enable_registry_skills",
    "smart_approve_default",
];

/// The line written above the added keys (English, no paths; it ships into the user's config).
const NOTE: &str =
    "# Vibe Work: Mistral's experiments do not switch these on (set one to true to opt in).";

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentPins {
    /// The keys added as `false`.
    pub added: Vec<&'static str>,
    /// The keys the config sets to something other than `false` (the person's own value, kept).
    pub kept_on: Vec<&'static str>,
    /// Why the file was left unchanged, when it was not valid TOML or the edit could not be verified.
    pub skipped: Option<String>,
}

impl ExperimentPins {
    fn skipped(why: &str) -> Self {
        Self {
            added: Vec::new(),
            kept_on: Vec::new(),
            skipped: Some(why.to_owned()),
        }
    }
}

/// The config text with every missing key of [`KEYS`] added as `false` at the top (top-level keys must come before
/// the first table), or `None` when nothing is added; and the outcome. Pure.
#[must_use]
pub fn with_pins_off(existing: &str) -> (Option<String>, ExperimentPins) {
    let Ok(before) = existing.parse::<Table>() else {
        return (
            None,
            ExperimentPins::skipped("config.toml is not valid TOML; Vibe reports the error itself"),
        );
    };
    let added: Vec<&'static str> = KEYS
        .into_iter()
        .filter(|key| !before.contains_key(*key))
        .collect();
    let kept_on = KEYS
        .into_iter()
        .filter(|key| {
            before
                .get(*key)
                .is_some_and(|v| v != &Value::Boolean(false))
        })
        .collect();
    let outcome = ExperimentPins {
        added,
        kept_on,
        skipped: None,
    };
    if outcome.added.is_empty() {
        return (None, outcome);
    }
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = String::with_capacity(existing.len() + 200);
    out.push_str(NOTE);
    out.push_str(newline);
    for key in &outcome.added {
        out.push_str(key);
        out.push_str(" = false");
        out.push_str(newline);
    }
    if !existing.is_empty() {
        out.push_str(newline);
        out.push_str(existing);
    }
    let verified = out.parse::<Table>().is_ok_and(|mut after| {
        outcome
            .added
            .iter()
            .all(|key| after.remove(*key) == Some(Value::Boolean(false)))
            && after == before
    });
    if !verified {
        return (
            None,
            ExperimentPins::skipped("the edit could not be verified"),
        );
    }
    (Some(out), outcome)
}

/// Makes `vibe_home/config.toml` set every key of [`KEYS`] (false unless already set). A missing file is created.
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
    let (updated, outcome) = with_pins_off(&existing);
    let Some(updated) = updated else {
        return Ok(outcome);
    };
    std::fs::create_dir_all(vibe_home).map_err(|e| format!("{}: {e}", vibe_home.display()))?;
    let tmp = vibe_home.join("config.toml.vw-tmp");
    std::fs::write(&tmp, &updated).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })?;
    Ok(outcome)
}
