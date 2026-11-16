//! Restores missing app-owned comments beside their settings after Vibe serializes its config.
//! Called by the startup config steps ([`crate::startup_config`]); values and every existing byte stay unchanged.

pub(crate) fn insert_missing(
    existing: &str,
    section: Option<&str>,
    key: &str,
    notes: &[&str],
) -> Option<String> {
    let mut current_section = None;
    let mut offset = 0;
    let mut anchor = None;
    for line in existing.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            current_section = Some(trimmed);
        } else if current_section == section
            && trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key)
        {
            anchor = Some(offset);
            break;
        }
        offset += line.len();
    }
    let anchor = anchor?;
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut prefix = String::new();
    for note in notes {
        if !existing.lines().any(|line| line == *note) {
            prefix.push_str(note);
            prefix.push_str(newline);
        }
    }
    if prefix.is_empty() {
        return None;
    }
    Some(format!(
        "{}{}{}",
        &existing[..anchor],
        prefix,
        &existing[anchor..]
    ))
}

/// The text with the missing notes restored above `key`, or `None` when nothing is missing or the notes would change
/// the config's data. Pure: [`crate::startup_config`] writes the result with the other startup fixes.
pub(crate) fn restored(
    existing: &str,
    section: Option<&str>,
    key: &str,
    notes: &[&str],
) -> Option<String> {
    let updated = insert_missing(existing, section, key, notes)?;
    (updated.parse::<toml::Table>().ok() == existing.parse::<toml::Table>().ok()).then_some(updated)
}
