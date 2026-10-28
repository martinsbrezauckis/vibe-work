//! Project Handbook Format 1.2 (`docs/public/HANDBOOK_FORMAT.md`): the format's vocabulary, reading a handbook
//! folder, the validator (§10), the explicit stylesheet installer and the Projects overview's head-only reads (§3).
//! Only `stylesheet::repair` writes, when the agent requests it; nothing needs a window or a runtime.
//!
//! Depends on: `page`, `lite`, `links`, `paths`, `html`. Called by: `hub`, `service`/`projects` (folder of a project),
//! `server` (Markdown frame), vw-core-api (hub, overview, `handbook_check`), tests.
//! Invariant: the folder of a project is `docs/handbook/` unless the user chose another one.

pub mod files;
pub mod format;
pub mod overview;
mod refs;
mod rules;
pub mod stylesheet;
pub mod validate;

/// The handbook folder of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Relative to the project folder, `/`-separated.
    pub relative: String,
}

/// The chosen handbook folder, or the standard folder when none was chosen.
#[must_use]
pub fn resolve_folder(saved: Option<&str>) -> Folder {
    Folder {
        relative: saved.unwrap_or(format::FOLDER).to_owned(),
    }
}
