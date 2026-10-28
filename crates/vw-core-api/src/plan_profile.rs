//! The app-owned `vw-plan` access mode keeps `VIBE_HOME/agents/vw-plan.toml` naming the plan
//! handbook folder of every project Vibe Work knows, and rewrites it BEFORE a session of a new project is opened (Vibe
//! reads custom profiles when a session opens: `_runtime.py:1485`).
//!
//! Depends on: vw-pages (`plan_profile_toml`, handbook folders), `core::Shared` (names, handbook settings),
//! `routing::needs_channel`.
//! Called by: `dispatch` (`forward`: before every call that opens a session; listings), `pages` (hub, settings).
//! Invariants:
//! - Written atomically (a new file, then renamed over the old one), only when its text changes.
//! - Projects come from Vibe's own session data (listing `cwd`, `session/start` `agentConfig.cwd`, the folder of a
//!   resumed or forked session), never from a page or a free-form client field; symlinked folders are added by their
//!   resolved path too (Vibe matches the path it resolves).
//! - The set only grows during a run (a removed project's entry stays until the next start); the union is the
//!   accepted residue: a `vw-plan` session may write another known project's `plans/` folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::core::Shared;

/// The profile's state.
pub(crate) struct PlanProfile {
    vibe_home: PathBuf,
    projects: Mutex<BTreeSet<String>>,
    written: Mutex<Option<String>>,
}

impl PlanProfile {
    /// A profile writer for `vibe_home` (nothing written yet).
    pub(crate) fn new(vibe_home: PathBuf) -> Self {
        Self {
            vibe_home,
            projects: Mutex::new(BTreeSet::new()),
            written: Mutex::new(None),
        }
    }

    /// The profile file.
    pub(crate) fn file(&self) -> PathBuf {
        self.vibe_home
            .join("agents")
            .join(format!("{}.toml", vw_pages::PLAN_PROFILE))
    }

    /// Adds folders; true when one was new.
    fn add(&self, folders: impl IntoIterator<Item = String>) -> bool {
        let mut projects = self.projects.lock().unwrap_or_else(PoisonError::into_inner);
        let mut added = false;
        for folder in folders.into_iter().filter(|f| !f.trim().is_empty()) {
            if let Ok(resolved) = std::fs::canonicalize(&folder) {
                let resolved = resolved.to_string_lossy().into_owned();
                // `canonicalize` on Windows answers `\\?\C:\...`; Vibe reports `C:\...`.
                let resolved = resolved
                    .strip_prefix(r"\\?\")
                    .map(str::to_owned)
                    .unwrap_or(resolved);
                added |= projects.insert(resolved);
            }
            added |= projects.insert(folder);
        }
        added
    }
}

/// The handbook folders whose `plans/` a project's `vw-plan` session may write: the folder the user chose, else
/// `docs/handbook`.
fn plan_folders(shared: &Shared, project: &str) -> Vec<String> {
    let settings = shared
        .pages
        .as_ref()
        .map(|hub| hub.service.settings(Path::new(project)));
    match settings {
        Some(s) if s.custom => vec![s.folder],
        _ => vec![vw_pages::DEFAULT_FOLDER.to_owned()],
    }
}

/// The profile text for the current set (handbook folders from each project's settings).
fn text(shared: &Shared) -> String {
    let profile = &shared.plan_profile;
    let projects: Vec<String> = profile
        .projects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .cloned()
        .collect();
    let targets: Vec<vw_pages::PlanTarget> = projects
        .into_iter()
        .flat_map(|project| {
            plan_folders(shared, &project)
                .into_iter()
                .map(move |pages_folder| vw_pages::PlanTarget {
                    project: project.clone(),
                    pages_folder,
                })
        })
        .collect();
    vw_pages::plan_profile_toml(&profile.vibe_home, &targets)
}

/// Writes the profile when its text changed. Failures are logged (the access mode then lacks the new project).
fn write(shared: &Shared) {
    let profile = &shared.plan_profile;
    let body = text(shared);
    let mut written = profile
        .written
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if written.as_deref() == Some(body.as_str()) {
        return;
    }
    let file = profile.file();
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let temp = file.with_extension("toml.vw-new");
            std::fs::write(&temp, &body)?;
            std::fs::rename(&temp, &file)
        });
    match result {
        Ok(()) => *written = Some(body),
        Err(error) => {
            tracing::warn!(%error, file = %file.display(), "the vw-plan profile could not be written");
        }
    }
}

/// Writes the profile at start (Vibe's own plans folder, no project yet), so the access mode exists.
pub(crate) fn start(shared: &Shared) {
    write(shared);
}

/// Rewrites the profile (a project's pages folder changed).
pub(crate) fn refresh(shared: &Shared) {
    write(shared);
}

/// Adds one project folder (writes when new).
pub(crate) fn note_project(shared: &Shared, folder: &str) {
    if shared.plan_profile.add([folder.to_owned()]) {
        write(shared);
    }
}

/// Adds the folders of a `session/list` page.
pub(crate) fn note_listing(shared: &Shared, items: &[Value]) {
    let folders = items
        .iter()
        .filter_map(|item| item.get("cwd").and_then(Value::as_str))
        .map(str::to_owned);
    if shared.plan_profile.add(folders) {
        write(shared);
    }
}

/// Before a call that opens a session: the folder that session will work in is in the profile.
pub(crate) fn before_open(shared: &Shared, method: &str, params: &Value) {
    let cwd_of = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .and_then(|id| shared.names.cwd_of(&shared.store, id))
    };
    let folder = match method {
        "session/start" => params
            .pointer("/agentConfig/cwd")
            .and_then(Value::as_str)
            .map(str::to_owned),
        "session/resume" | "session/continue" => cwd_of("sessionId"),
        "session/fork" => cwd_of("sourceSessionId"),
        _ => crate::routing::needs_channel(method, params)
            .and_then(|id| shared.names.cwd_of(&shared.store, id)),
    };
    if let Some(folder) = folder {
        note_project(shared, &folder);
    }
}

/// Every project folder known in this run (the workspace hub).
pub(crate) fn projects(shared: &Shared) -> Vec<String> {
    let set = shared
        .plan_profile
        .projects
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // A resolved duplicate of a symlinked folder is shown once (by the folder Vibe reports).
    let mut seen = BTreeSet::new();
    set.iter()
        .filter(|p| {
            let key = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p.as_str()));
            seen.insert(key)
        })
        .cloned()
        .collect()
}
