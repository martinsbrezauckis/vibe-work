//! Per-project git settings: the commit message instruction, the credit-line switch and the "Run this
//! repository's commit hooks" switch, kept per repository (the main working tree's root, so a session copy and
//! the folder share them) in one user-only JSON file in the app data (`<changes dir>/git/settings.json`).
//!
//! Depends on: vw-platform (`replace_private`), `git_cmd::GitHub` (the folder), `git_view::GitSettings`. Called by:
//! `git_cmd`.
//! Invariants: the file is replaced atomically; an unreadable file reads as defaults (logged), never guessed;
//! keys are the repository root as text.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{CoreError, ErrorCode};
use crate::git_cmd::GitHub;
use crate::git_view::GitSettings;

const FILE: &str = "settings.json";

fn read_all(git: &GitHub) -> BTreeMap<String, GitSettings> {
    let Some(root) = git.root.as_ref() else {
        return BTreeMap::new();
    };
    match std::fs::read(root.join(FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(%error, "git settings unreadable: defaults used");
            BTreeMap::new()
        }),
        Err(_) => BTreeMap::new(),
    }
}

/// The settings of the repository at `main_root` (defaults when none were saved).
pub(crate) fn load(git: &GitHub, main_root: &Path) -> GitSettings {
    read_all(git)
        .remove(&main_root.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Saves the settings of the repository at `main_root`.
///
/// # Errors
/// `internal` when the file cannot be written.
pub(crate) fn save(
    git: &GitHub,
    main_root: &Path,
    settings: &GitSettings,
) -> Result<(), CoreError> {
    let folder = git.folder(".")?;
    let mut all = read_all(git);
    all.insert(main_root.to_string_lossy().into_owned(), settings.clone());
    let bytes = serde_json::to_vec_pretty(&all)
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("git settings: {e}")))?;
    vw_platform::replace_private(&folder.join(FILE), &bytes)
        .map_err(|e| CoreError::new(ErrorCode::Internal, format!("git settings: {e}")))
}
