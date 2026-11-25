//! Takes out the Mistral Medium 3.5 row that Vibe Work 0.1.16 and 0.1.17 copied into the app's Vibe config.
//! Medium 3.5 is a built-in row of Vibe 3.1.4 that Mistral's own model list leaves out; those versions kept the row
//! alive for a chat that had saved it. It is no longer offered, so the row goes: a chat that saved it takes the
//! ordinary "unavailable" notice and the Settings default (`restore`).
//! Only what the app itself wrote is removed. The marker file in the app's sessions folder, which only those
//! versions wrote, is the proof; without it config.toml is never read. The row must still carry exactly the facts
//! that were written, so a row a person edited or added themselves stays. The marker goes once the row is handled;
//! a failure keeps it and the next start tries again.
//! Called by `core_start::start_supervisor`, before Vibe launches, once per start.

use std::path::Path;

use toml_edit::{DocumentMut, Item};

use crate::error::{CoreError, ErrorCode};

const MARKER: &str = "built-in-medium-choice-v1";
/// The `[[models]]` row exactly as 0.1.16 and 0.1.17 wrote it (Vibe 3.1.4's own built-in definition).
const ROW: &str = r#"
[[models]]
name = "mistral-vibe-cli-latest"
provider = "mistral"
alias = "mistral-medium-3.5"
display_name = "Mistral Medium 3.5"
temperature = 1.0
input_price = 1.5
output_price = 7.5
cached_input_price = 0.15
thinking = "high"
thinking_levels = ["off", "high"]
supports_images = true
max_context_length = 262144
"#;

fn failed() -> CoreError {
    CoreError::new(
        ErrorCode::Internal,
        "The Medium 3.5 row an earlier version kept could not be removed from Vibe's configuration.",
    )
}

/// `before` without every row that equals the written one, `None` when it has none.
fn without_kept_row(before: &str) -> Result<Option<String>, CoreError> {
    let written: toml::Table = ROW.parse().map_err(|_| failed())?;
    let written = written.get("models").and_then(toml::Value::as_array);
    let parsed: toml::Table = before.parse().map_err(|_| failed())?;
    let Some(rows) = parsed.get("models").and_then(toml::Value::as_array) else {
        return Ok(None);
    };
    let found: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| written.and_then(|written| written.first()) == Some(*row))
        .map(|(index, _)| index)
        .collect();
    if found.is_empty() {
        return Ok(None);
    }
    let mut document: DocumentMut = before.parse().map_err(|_| failed())?;
    // A file Vibe rewrote with inline rows is not ours to reshape: it stays and the marker waits.
    let tables = document
        .get_mut("models")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(failed)?;
    for index in found.into_iter().rev() {
        tables.remove(index);
    }
    let after = document.to_string();
    after.parse::<toml::Table>().map_err(|_| failed())?;
    Ok(Some(after))
}

/// Removes the kept row from `config`; true when the file was changed.
fn retire_row(config: &Path) -> Result<bool, CoreError> {
    if !config.exists() {
        return Ok(false);
    }
    let before = crate::default_model::read(config)?;
    let Some(after) = without_kept_row(&before)? else {
        return Ok(false);
    };
    // Vibe or a person may be writing the file: replace only the bytes that were read.
    if crate::default_model::read(config)? != before {
        return Err(failed());
    }
    vw_platform::replace_private(config, after.as_bytes()).map_err(|_| failed())?;
    Ok(true)
}

/// Retires the row an earlier version kept, if this install has one.
pub(crate) fn retire_kept_medium(home: &Path, sessions: Option<&Path>) {
    let Some(sessions) = sessions else { return };
    let marker = sessions.join(MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            tracing::warn!("the kept Medium 3.5 marker is not a plain file and was left alone");
            return;
        }
        Err(_) => return,
    }
    match retire_row(&home.join("config.toml")) {
        Ok(changed) => match std::fs::remove_file(&marker) {
            Ok(()) => tracing::debug!(changed, "the kept Medium 3.5 row is retired"),
            Err(error) => tracing::warn!(%error, "the kept Medium 3.5 marker could not be removed"),
        },
        Err(error) => {
            tracing::warn!(%error, "the kept Medium 3.5 row will be retried at the next start");
        }
    }
}

#[cfg(test)]
#[path = "kept_medium_tests.rs"]
mod tests;
