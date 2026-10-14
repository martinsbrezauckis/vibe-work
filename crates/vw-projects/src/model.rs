//! The project store's public types: a project, the snapshot both clients read, the inputs of the filing rules and
//! the errors.
//!
//! Depends on: serde. Called by: `state`, `store`, vw-core-api (wire types, generated TypeScript).
//! Invariants: wire form camelCase; a member value `None` means "seen, not in a project".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Most projects (a guard against a runaway client; the filing rule files into "no project" past it).
pub const MAX_PROJECTS: usize = 1000;
/// Most remembered sessions (a session past it stays unseen and shows as "not in a project").
pub const MAX_MEMBERS: usize = 200_000;
/// Longest project name, in characters.
pub const MAX_NAME_CHARS: usize = 120;

/// One project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// `p` + 16 random hex digits.
    pub id: String,
    /// The user's name for it (cleaned, 1-120 characters).
    pub name: String,
    /// The folder whose handbook the project shows; `None` for a name-only project.
    pub main_folder: Option<String>,
    /// Created by the filing rule for a folder (not by the user).
    pub auto: bool,
    /// When it was created, ms since the Unix epoch.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_ms: u64,
}

/// What both clients read: the projects in the user's order and the session -> project map.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Projects, in the user's order.
    pub projects: Vec<Project>,
    /// Session id -> project id; `None` = seen, not in a project.
    pub members: BTreeMap<String, Option<String>>,
}

/// Where a new session must go, decided by whoever started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// Into this project (ignored when it no longer exists).
    Project(String),
    /// Into no project.
    Unfiled,
}

/// A session Vibe Work saw being created (`session/start`, a fork, a rewind that made a new id).
#[derive(Debug, Clone, Default)]
pub struct NewSession<'a> {
    /// The new session's id.
    pub id: &'a str,
    /// The starter's choice, if any (rule 1).
    pub hint: Option<Hint>,
    /// The session it came from (fork, rewind, continue-in-new), whose filing it inherits (rule 2).
    pub source: Option<&'a str>,
    /// Its folder for the folder rule (rule 3): the folder the start asked for, else the repository of its copy,
    /// else its `cwd`.
    pub folder: Option<&'a str>,
}

/// A session seen in a listing (or resumed) for which only its folder is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// Session id.
    pub id: String,
    /// Its folder for the folder rule.
    pub folder: Option<String>,
}

/// What a sweep did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepOutcome {
    /// Sessions filed for the first time.
    pub filed: usize,
    /// Members not listed and not loaded, kept (first miss, or missed for less than `DROP_AFTER_MS`).
    pub missing: usize,
    /// Members dropped (missed by two sweeps at least `DROP_AFTER_MS` apart).
    pub dropped: usize,
}

/// Why a project operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectsError {
    /// A bad name, folder, order or a full store.
    #[error("{0}")]
    Invalid(String),
    /// No project (or no known session) with this id.
    #[error("{0}")]
    NotFound(String),
    /// The store file could not be written; nothing changed.
    #[error("the projects could not be saved: {0}")]
    Io(String),
}
