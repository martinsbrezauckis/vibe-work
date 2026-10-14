//! Project names and folders: cleaning a name the user typed, the name an auto project gets from its folder, and the
//! folder form the rules compare.
//!
//! Depends on: std. Called by: `state`, vw-core-api (dialog text, the Debug API's folder name).
//! Invariants: the auto name of a folder follows the session list's old rule (last folder name; `parent/last` when
//! another project or another new folder has the same last name), so the first run shows the names users knew.

use std::collections::BTreeSet;

use crate::model::{MAX_NAME_CHARS, ProjectsError};

/// A name as stored: control characters dropped, whitespace collapsed, trimmed, at most [`MAX_NAME_CHARS`].
///
/// # Errors
/// [`ProjectsError::Invalid`] when nothing is left or the name is longer than the limit.
pub fn clean_name(text: &str) -> Result<String, ProjectsError> {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let name = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(ProjectsError::Invalid("a project needs a name".to_owned()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ProjectsError::Invalid(format!(
            "a project name has at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name)
}

/// The folder as it is kept and shown: trailing separators removed (a root such as `/` or `C:\` stays). Rules compare
/// folders with [`same_folder`].
#[must_use]
pub fn normalize_folder(folder: &str) -> String {
    let trimmed = folder.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        folder.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// True when `a` and `b` name one folder on this platform: on Windows case, `/` vs `\` and
/// the `\\?\` prefix do not matter, on macOS case does not (`vw_platform::folder_key`). Two spellings of one folder
/// (a CLI session started from `c:\dev\app`, the app's `C:\Dev\app`) file into one project.
#[must_use]
pub fn same_folder(a: &str, b: &str) -> bool {
    vw_platform::folder_key(a) == vw_platform::folder_key(b)
}

fn segments(folder: &str) -> Vec<&str> {
    folder
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect()
}

/// The folder's last name (`"/home/u/app"` -> `"app"`); the whole text when it has no separator.
#[must_use]
pub fn folder_label(folder: &str) -> String {
    segments(folder)
        .last()
        .map_or_else(|| folder.to_owned(), |s| (*s).to_owned())
}

fn with_parent(folder: &str) -> String {
    let parts = segments(folder);
    match parts.as_slice() {
        [.., parent, last] => format!("{parent}/{last}"),
        _ => folder_label(folder),
    }
}

/// The name an auto project for `folder` gets. `taken` = names of the existing projects; `folders` = their main
/// folders; `batch` = the folders filed in the same step (a listing page), so two new `app` folders both get their
/// parent, and a new `app` folder next to an existing project for another `app` folder gets its parent too.
pub(crate) fn auto_name(
    folder: &str,
    taken: &BTreeSet<String>,
    folders: &[&str],
    batch: &[String],
) -> String {
    let last = folder_label(folder);
    let clash = taken.contains(&last)
        || folders
            .iter()
            .any(|other| !same_folder(other, folder) && folder_label(other) == last)
        || batch
            .iter()
            .filter(|other| !same_folder(other, folder) && folder_label(other) == last)
            .count()
            > 0;
    let base = if clash { with_parent(folder) } else { last };
    let mut name: String = base.chars().take(MAX_NAME_CHARS - 6).collect();
    let mut n = 2;
    while taken.contains(&name) {
        name = format!(
            "{} ({n})",
            base.chars().take(MAX_NAME_CHARS - 6).collect::<String>()
        );
        n += 1;
    }
    name
}
