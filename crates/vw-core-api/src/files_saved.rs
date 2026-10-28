//! Reads one checked file from the last commit (or, for Files > History, from a named commit) for the editor
//! comparison. Uses the bounded Git runner; called by files after checking the session handle again and by
//! `explorer_history`. Reads raw blob bytes without filters, hooks or changes to the working tree.
use crate::files_read::TEXT_LIMIT;
use crate::files_view::FilesResult;
use std::path::Path;
use vw_changes::git::{Call, Index};

fn unavailable(note: &str) -> FilesResult {
    FilesResult::Saved {
        text: None,
        note: Some(note.to_owned()),
    }
}

pub(crate) fn read(path: &Path) -> FilesResult {
    let Some(parent) = path.parent() else {
        return unavailable("This file has no version history.");
    };
    let Ok(Some(repo)) = vw_changes::discover(parent) else {
        return unavailable("This folder has no readable version history.");
    };
    // Git names the root as `C:/...` on Windows while the checked file is held in its canonical `\\?\C:\...`
    // spelling (or a short name, another case): placed in the repository by comparing canonical forms.
    let Some(relative) = vw_changes::repository_relative(&repo.root, path) else {
        return unavailable("This file is outside the repository.");
    };
    let name = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    match blob_text(
        &repo.root,
        &format!("HEAD:{name}"),
        "This file has no version in the last commit.",
    ) {
        (Some(text), _) => FilesResult::Saved {
            text: Some(text),
            note: None,
        },
        (None, note) => FilesResult::Saved { text: None, note },
    }
}

/// The text of the blob `spec` (`<commit>:<path>`) read in `dir`, or the reason in the user's words.
pub(crate) fn blob_text(dir: &Path, spec: &str, missing: &str) -> (Option<String>, Option<String>) {
    let no = |note: &str| (None, Some(note.to_owned()));
    let size = Call::new(dir, Index::User)
        .args(["cat-file", "-s", spec])
        .max_output(128)
        .output();
    let Ok(size) = size else {
        return no("The saved version could not be read. Try again.");
    };
    if !size.ok() {
        return no(missing);
    }
    let size = String::from_utf8_lossy(&size.stdout).trim().parse::<u64>();
    if !matches!(size, Ok(n) if n <= TEXT_LIMIT) {
        return no("The saved version is too large to compare here.");
    }
    let bytes = Call::new(dir, Index::User)
        .args(["cat-file", "blob", spec])
        .max_output(usize::try_from(TEXT_LIMIT).unwrap_or(256 * 1024) + 1)
        .run();
    let Ok(bytes) = bytes else {
        return no("The saved version could not be read. Try again.");
    };
    if bytes.contains(&0) {
        return no("The saved version has no editable text.");
    }
    match String::from_utf8(bytes) {
        Ok(text) => (
            Some(
                text.replace("\r\n", "\n")
                    .trim_start_matches('\u{feff}')
                    .to_owned(),
            ),
            None,
        ),
        Err(_) => no("The saved version has no editable text."),
    }
}
