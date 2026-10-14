//! Project Handbook Format 1.2 (`docs/public/HANDBOOK_FORMAT.md`): the format's vocabulary, reading a handbook
//! folder, the validator (§10), the explicit stylesheet installer and the Projects overview's head-only reads (§3).
//! Only `stylesheet::repair` writes, when the agent requests it; nothing needs a window or a runtime.
//!
//! Depends on: `page`, `lite`, `links`, `paths`, `html`. Called by: `hub`, `service`/`projects` (folder of a project),
//! `server` (Markdown frame), vw-core-api (hub, overview, `handbook_check`), tests.
//! Invariant: the folder of a project is `docs/handbook/` unless the user chose another one; a project that has
//! only the pre-1.0 `docs/pages/` is read from there and flagged for migration (§2, §11).

pub mod files;
pub mod format;
pub mod overview;
mod refs;
mod rules;
pub mod stylesheet;
pub mod validate;

use std::path::Path;

/// The handbook folder of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Relative to the project folder, `/`-separated.
    pub relative: String,
    /// The pre-1.0 folder (`docs/pages/`) is read because `docs/handbook/` does not exist (§2).
    pub legacy: bool,
}

/// The handbook folder of `project`: `saved` (a folder the user chose) when given; else `docs/handbook` when it
/// exists or `docs/pages` does not; else `docs/pages` (legacy).
#[must_use]
pub fn resolve_folder(project: &Path, saved: Option<&str>) -> Folder {
    if let Some(folder) = saved {
        return Folder {
            relative: folder.to_owned(),
            legacy: false,
        };
    }
    let join = |rel: &str| rel.split('/').fold(project.to_path_buf(), |p, s| p.join(s));
    if !join(format::FOLDER).is_dir() && join(format::LEGACY_FOLDER).is_dir() {
        return Folder {
            relative: format::LEGACY_FOLDER.to_owned(),
            legacy: true,
        };
    }
    Folder {
        relative: format::FOLDER.to_owned(),
        legacy: false,
    }
}
