//! The model Vibe writes session titles with: Vibe Work's own `VIBE_HOME/config.toml` gets
//! `[utility_models] title = "active"` unless it already names a title model, so each chat's own model (the one the
//! person picked from Mistral's model list) writes its title.
//!
//! Why: Vibe Work names no model of its own here. A model name chosen by the app breaks when Mistral renames or
//! retires that model; `active` is Vibe's selector for the session's active model and follows whatever the chat uses.
//! Without any `[utility_models]` title Vibe would pick a model itself (a fast model it probes for, else the active
//! one); `active` keeps the title on the model the person chose.
//!
//! Earlier versions wrote `title = "mistral-medium-3.5"`. That exact app seed is replaced by `active` once
//! ([`MIGRATION`] marker in the managed home); after the marker a title model the person names, including that one,
//! is kept. Vibe rewrites the config without comments, so the seed is recognised by its exact value only.
//!
//! Depends on: `toml` (parsing only; the file is edited as text so the user's comments and layout stay).
//! Called by: [`crate::launch::ProductLaunch`] and [`crate::launch::DevLaunch`] before the server starts.
//! Invariants:
//! - Only Vibe Work's own `VIBE_HOME` is touched (never `~/.vibe`; the CLI-sharing switch shares sessions, not
//!   the config).
//! - A file that is not valid TOML, or that writes `utility_models` as dotted keys or an inline table, is left
//!   byte-identical (Skipped).
//! - The edit is checked before it is written: the new file must parse, name the title model, and equal the old
//!   file's data plus (or with) that one key. The write goes through a temporary file and a rename, together with the
//!   other startup fixes ([`crate::startup_config`]).

use std::path::Path;

use toml::{Table, Value};

/// The title model we set: Vibe's selector for the session's active model (`ACTIVE_MODEL_SELECTOR`).
pub const TITLE_MODEL_ALIAS: &str = "active";

/// The title model earlier versions wrote; replaced by [`TITLE_MODEL_ALIAS`] once.
pub const OLD_SEED_ALIAS: &str = "mistral-medium-3.5";

/// App-owned marker in the managed home (outside the config Vibe rewrites): the old seed was replaced once.
pub const MIGRATION: &str = ".vw-title-model-active-v1";

/// The lines written above the added table (English, no paths; they ship into the user's config).
const NOTE: [&str; 2] = [
    "# Vibe Work: each chat's own model writes its session title.",
    "# Name a model alias here to title every chat with that model instead.",
];

/// The notes earlier versions wrote beside [`OLD_SEED_ALIAS`]; removed with it.
const OLD_NOTE: [&str; 2] = [
    "# Vibe Work: session titles are written by Mistral Medium 3.5, whatever model a session uses.",
    "# Write title = \"active\" to title sessions with their own model instead.",
];

/// What [`ensure`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleModel {
    /// `title = "active"` was added (a new `[utility_models]` table, or a key in the existing one).
    Added,
    /// The earlier app seed `title = "mistral-medium-3.5"` was replaced by `title = "active"` (once).
    Migrated,
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
            Self::Migrated => format!(
                "session titles: earlier app title model {OLD_SEED_ALIAS} replaced by {TITLE_MODEL_ALIAS}"
            ),
            Self::Kept(alias) => format!("session titles: config names {alias:?}, kept"),
            Self::Skipped(why) => format!("session titles: config not changed ({why})"),
        }
    }
}

/// Makes `vibe_home/config.toml` name a title model (see the module docs), through the guarded startup write
/// ([`crate::startup_config`]: never through a link, never a file over 1 MiB or one that changed meanwhile). A missing
/// file is created with only our table (Vibe 2.26.1 writes no `config.toml` on its own; a later `config/write` keeps
/// our lines).
///
/// # Errors
/// The I/O error text when the file cannot be read or written.
pub fn ensure(vibe_home: &Path) -> Result<TitleModel, String> {
    let mut draft = crate::startup_config::Draft::open(vibe_home)
        .map_err(|e| format!("{}: {e}", vibe_home.join("config.toml").display()))?;
    let pending = migration_pending(vibe_home)?;
    let staged = draft.apply(|text, _| edit(text, pending), TitleModel::Skipped);
    let written = draft.commit();
    let outcome = written.settle(staged, TitleModel::Skipped)?;
    finish(vibe_home, pending, &outcome, written.landed())?;
    Ok(outcome)
}

/// Whether the one-time replacement of the old seed is still to run (its marker is absent).
///
/// # Errors
/// The marker could not be checked.
pub(crate) fn migration_pending(vibe_home: &Path) -> Result<bool, String> {
    let marker = vibe_home.join(MIGRATION);
    marker
        .try_exists()
        .map(|exists| !exists)
        .map_err(|e| format!("{}: {e}", marker.display()))
}

/// Records the one-time replacement once the file holds this step's result (written, or nothing to write) and the
/// step was not skipped: a refused, failed or invalid config must not consume it.
///
/// # Errors
/// The marker could not be written.
pub(crate) fn finish(
    vibe_home: &Path,
    pending: bool,
    outcome: &TitleModel,
    landed: bool,
) -> Result<(), String> {
    if !pending || !landed || matches!(outcome, TitleModel::Skipped(_)) {
        return Ok(());
    }
    let marker = vibe_home.join(MIGRATION);
    std::fs::write(&marker, b"1\n").map_err(|e| format!("{}: {e}", marker.display()))
}

/// This step's edit for [`crate::startup_config`]: [`with_title_model`], the old seed replaced (only while
/// `migrate`), or our missing notes restored beside our own title model. The earlier version's notes are replaced
/// by ours in place whatever the outcome (they name a model the app no longer sets).
pub(crate) fn edit(existing: &str, migrate: bool) -> (Option<String>, TitleModel) {
    let renoted = renoted(existing);
    let text = renoted.as_deref().unwrap_or(existing);
    let (edited, outcome) = match with_title_model(text) {
        Ok(text) => (Some(text), TitleModel::Added),
        Err(TitleModel::Kept(alias)) if migrate && alias == OLD_SEED_ALIAS => {
            let Some(text) = without_old_seed(text) else {
                let why = "the earlier title model line could not be replaced";
                return (None, TitleModel::Skipped(why.to_owned()));
            };
            (Some(text), TitleModel::Migrated)
        }
        Err(outcome @ TitleModel::Skipped(_)) => return (None, outcome),
        Err(outcome) => {
            let notes = if matches!(&outcome, TitleModel::Kept(alias) if alias == TITLE_MODEL_ALIAS)
            {
                crate::config_comments::restored(text, Some("[utility_models]"), "title", &NOTE)
            } else {
                None
            };
            (notes, outcome)
        }
    };
    (edited.or(renoted), outcome)
}

/// `existing` with each line of the earlier [`OLD_NOTE`] replaced by the matching line of [`NOTE`] (comments only;
/// the data is unchanged), or `None` when it holds none.
fn renoted(existing: &str) -> Option<String> {
    let mut out = String::with_capacity(existing.len());
    let mut changed = false;
    for line in existing.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        if let Some(i) = OLD_NOTE.iter().position(|old| *old == bare) {
            out.push_str(NOTE[i]);
            out.push_str(&line[bare.len()..]);
            changed = true;
        } else {
            out.push_str(line);
        }
    }
    changed.then_some(out)
}

/// `existing` with the old seed's line in `[utility_models]` replaced by `title = "active"`, then our notes restored
/// when missing; `None` when the line is not found or the result would change anything else.
fn without_old_seed(existing: &str) -> Option<String> {
    let before: Table = existing.parse().ok()?;
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = String::with_capacity(existing.len());
    let mut in_table = false;
    let mut replaced = false;
    for line in existing.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        if bare.trim_start().starts_with('[') {
            in_table = is_header(line);
        } else if in_table && !replaced && is_old_seed_line(bare) {
            out.push_str("title = \"");
            out.push_str(TITLE_MODEL_ALIAS);
            out.push('"');
            // A comment of the person's own stays; the earlier app comment names the old model and goes.
            if let Some((_, comment)) = bare.split_once('#')
                && !comment.trim_start().starts_with("added by Vibe Work")
            {
                out.push_str(" #");
                out.push_str(comment);
            }
            out.push_str(if line.ends_with('\n') { newline } else { "" });
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        return None;
    }
    let out = crate::config_comments::restored(&out, Some("[utility_models]"), "title", &NOTE)
        .unwrap_or(out);
    let mut expected = before;
    if let Some(Value::Table(utility)) = expected.get_mut("utility_models") {
        utility.insert("title".into(), Value::String(TITLE_MODEL_ALIAS.into()));
    }
    (out.parse::<Table>().ok()? == expected).then_some(out)
}

/// Whether `line` is the `title` key holding exactly the old seed (any spacing, any trailing comment).
fn is_old_seed_line(line: &str) -> bool {
    let Some((key, rest)) = line.split_once('=') else {
        return false;
    };
    if key.trim() != "title" {
        return false;
    }
    let value = rest.split('#').next().unwrap_or_default().trim();
    value == format!("\"{OLD_SEED_ALIAS}\"") || value == format!("'{OLD_SEED_ALIAS}'")
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
            out.push_str("  # added by Vibe Work: each chat's own model writes its title");
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
