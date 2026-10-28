//! The model Vibe writes session titles with: Vibe Work's own `VIBE_HOME/config.toml`
//! gets `[utility_models] title = "mistral-medium-3.5"` unless it already names a title model.
//!
//! Why: Vibe Work names the title model, so session titles come from one known model whatever model a session
//! runs: Mistral Medium 3.5, which answers Vibe's title request (Vibe 3 samples it at the model entry's temperature,
//! 1.0 for Medium 3.5; 2.26.1 sent greedy `temperature = 0`). Without a `[utility_models]` title model Vibe writes
//! titles with the session's active model.
//!
//! Why this and not `top_p` on the model entry: a model's `top_p` applies to the session's answers, not to the
//! title request, so it would not change titles, and it would change the sampling of every normal answer on that
//! model. Settings' per-model "Sampling (top p)" row therefore stays what it says: it applies to answers, not to
//! titles.
//!
//! What it changes: only which model writes titles. Medium 3.5 is Vibe's built-in default model (always listed
//! unless the user's `allowed_models` hides it; then Vibe warns and falls back to the active model, as before) and
//! is a Mistral model, so titles go to the same provider as any Mistral session. A user who wants titles from the
//! session's own model writes `title = "active"`; any title model the user names is left as it is.
//!
//! Revisit this default on each Vibe update.
//!
//! Depends on: `toml` (parsing only; the file is edited as text so the user's comments and layout stay).
//! Called by: [`crate::launch::ProductLaunch`] and [`crate::launch::DevLaunch`] before the server starts.
//! Invariants:
//! - Only Vibe Work's own `VIBE_HOME` is touched (never `~/.vibe`; the CLI-sharing switch shares sessions, not
//!   the config).
//! - A file that is not valid TOML, or that writes `utility_models` as dotted keys or an inline table, is left
//!   byte-identical (Skipped).
//! - The edit is checked before it is written: the new file must parse, name our title model, and equal the old
//!   file's data plus that one key. The write goes through a temporary file and a rename.

use std::path::Path;

use toml::{Table, Value};

/// The title model alias we set: Vibe's built-in default model, Mistral Medium 3.5 (`vibe_schema.py`
/// `DEFAULT_ACTIVE_MODEL_CONFIG`), which accepts Vibe's title request (2.26.1's greedy one too).
pub const TITLE_MODEL_ALIAS: &str = "mistral-medium-3.5";

/// The lines written above the added table (English, no paths; they ship into the user's config).
const NOTE: [&str; 2] = [
    "# Vibe Work: session titles are written by Mistral Medium 3.5, whatever model a session uses.",
    "# Write title = \"active\" to title sessions with their own model instead.",
];

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleModel {
    /// `title = "mistral-medium-3.5"` was added (a new `[utility_models]` table, or a key in the existing one).
    Added,
    /// The config already names a title model (the user's, or ours from an earlier start): left as it is.
    Kept(String),
    /// Left unchanged, with the reason (not valid TOML, or a `utility_models` layout we do not edit).
    Skipped(String),
}

impl TitleModel {
    /// One line for the log.
    #[must_use]
    pub fn log_line(&self) -> String {
        match self {
            Self::Added => format!("session titles: title model set to {TITLE_MODEL_ALIAS}"),
            Self::Kept(alias) => format!("session titles: config names {alias:?}, kept"),
            Self::Skipped(why) => format!("session titles: config not changed ({why})"),
        }
    }
}

/// Makes `vibe_home/config.toml` name a title model (see the module docs). A missing file is created with only our
/// table (Vibe 2.26.1 writes no `config.toml` on its own; a later `config/write` keeps our lines).
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<TitleModel, String> {
    let path = vibe_home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let updated = match with_title_model(&existing) {
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
    Ok(TitleModel::Added)
}

/// The config text with our title model added, or the outcome when nothing is to be written. Pure (the tests and
/// [`ensure`] share it).
///
/// # Errors
/// `Kept` / `Skipped` when the text stays as it is.
pub fn with_title_model(existing: &str) -> Result<String, TitleModel> {
    let table: Table = existing.parse().map_err(|_| {
        TitleModel::Skipped(
            "config.toml is not valid TOML; Vibe reports the error itself".to_owned(),
        )
    })?;
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let key = format!("title = \"{TITLE_MODEL_ALIAS}\"");
    let updated = match table.get("utility_models") {
        Some(Value::Table(utility)) => {
            if let Some(title) = utility.get("title") {
                return Err(TitleModel::Kept(
                    title
                        .as_str()
                        .map_or_else(|| title.to_string(), str::to_owned),
                ));
            }
            insert_after_header(existing, &key, newline).ok_or_else(|| {
                TitleModel::Skipped(
                    "utility_models is written as dotted keys or an inline table".to_owned(),
                )
            })?
        }
        Some(_) => {
            return Err(TitleModel::Skipped(
                "utility_models is not a table".to_owned(),
            ));
        }
        None => append_table(existing, &key, newline),
    };
    if verified(&table, &updated) {
        Ok(updated)
    } else {
        Err(TitleModel::Skipped(
            "the edit could not be verified".to_owned(),
        ))
    }
}

/// `existing` plus a blank line, [`NOTE`] and a new `[utility_models]` table holding `key`.
fn append_table(existing: &str, key: &str, newline: &str) -> String {
    let mut out = existing.to_owned();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(newline);
        }
        out.push_str(newline);
    }
    for line in NOTE {
        out.push_str(line);
        out.push_str(newline);
    }
    out.push_str("[utility_models]");
    out.push_str(newline);
    out.push_str(key);
    out.push_str(newline);
    out
}

/// `existing` with `key` on the line after its `[utility_models]` header (spaces inside the brackets and a trailing
/// comment allowed), or `None` when there is no such header line.
fn insert_after_header(existing: &str, key: &str, newline: &str) -> Option<String> {
    let mut out = String::with_capacity(existing.len() + key.len() + 64);
    let mut done = false;
    for line in existing.split_inclusive('\n') {
        out.push_str(line);
        if !done && is_header(line) {
            if !line.ends_with('\n') {
                out.push_str(newline);
            }
            out.push_str(key);
            out.push_str("  # added by Vibe Work: session titles by Mistral Medium 3.5");
            out.push_str(newline);
            done = true;
        }
    }
    done.then_some(out)
}

/// Whether `line` is the `[utility_models]` table header.
fn is_header(line: &str) -> bool {
    let code = line.split('#').next().unwrap_or_default();
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    compact == "[utility_models]"
}

/// The edit added exactly `utility_models.title = TITLE_MODEL_ALIAS` and nothing else.
fn verified(before: &Table, updated: &str) -> bool {
    let Ok(mut after) = updated.parse::<Table>() else {
        return false;
    };
    let Some(Value::Table(utility)) = after.get_mut("utility_models") else {
        return false;
    };
    if utility.remove("title").as_ref().and_then(Value::as_str) != Some(TITLE_MODEL_ALIAS) {
        return false;
    }
    if utility.is_empty() && !before.contains_key("utility_models") {
        after.remove("utility_models");
    }
    &after == before
}
