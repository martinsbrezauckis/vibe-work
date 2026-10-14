//! The pure project model and every rule (B-F): user operations, first-sight filing and the
//! reconcile sweep. No I/O; the store persists what this produces.
//!
//! Depends on: `model`, `names`, getrandom (ids). Called by: `store`, the private tests.
//! Invariants:
//! - `delete` only re-files members to `None`; no operation removes a member except `forget` (Vibe deleted the
//!   session) and the second strike of `sweep`.
//! - A session is filed once, at first sight; later sightings never change its project.
//! - Projects keep the user's order; a new session's auto project and a user-created project go to the top, an auto
//!   project for an old session seen in a listing goes to the end (Vibe lists newest first, so the first run keeps the
//!   order the list showed before projects existed).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{
    Hint, MAX_MEMBERS, MAX_PROJECTS, NewSession, Project, ProjectsError, Seen, Snapshot,
    SweepOutcome,
};
use crate::names::{auto_name, clean_name, normalize_folder, same_folder};

/// Most session ids one `move` may carry.
pub const MAX_MOVE: usize = 500;

/// How long after its first miss a member must still be missing before a sweep drops it (10 minutes). Two sweeps
/// close together can both skip a session that is updated while the list is paged; ten minutes apart they do not.
pub const DROP_AFTER_MS: u64 = 10 * 60 * 1000;

/// The whole stored state (the store file's content).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) projects: Vec<Project>,
    #[serde(default)]
    pub(crate) members: BTreeMap<String, Option<String>>,
    /// Members missing from one sweep (first strike), with when.
    #[serde(default)]
    pub(crate) missing: BTreeMap<String, u64>,
}

fn new_id(now: u64) -> String {
    use std::fmt::Write as _;
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Never seen in practice; the time keeps ids distinct enough for a local store.
        bytes = now.to_le_bytes();
    }
    bytes.iter().fold(String::from("p"), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn not_found(id: &str) -> ProjectsError {
    ProjectsError::NotFound(format!("there is no project with the id {id:.40}"))
}

impl State {
    /// The current file format.
    pub const VERSION: u32 = 1;

    /// What the clients read.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            projects: self.projects.clone(),
            members: self.members.clone(),
        }
    }

    /// The projects in the user's order.
    #[must_use]
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// Session id -> project id (`None`: seen, not in a project).
    #[must_use]
    pub fn members(&self) -> &BTreeMap<String, Option<String>> {
        &self.members
    }

    /// The project with `id`.
    #[must_use]
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    /// Where `session` is filed: `None` = never seen, `Some(None)` = not in a project.
    #[must_use]
    pub fn filing_of(&self, session: &str) -> Option<Option<&str>> {
        self.members.get(session).map(Option::as_deref)
    }

    /// How many sessions are filed in `project`.
    #[must_use]
    pub fn member_count(&self, project: &str) -> usize {
        self.members
            .values()
            .filter(|p| p.as_deref() == Some(project))
            .count()
    }

    /// Sessions marked missing by the last sweep (first strike).
    #[must_use]
    pub fn missing(&self) -> Vec<String> {
        self.missing.keys().cloned().collect()
    }

    fn index(&self, id: &str) -> Result<usize, ProjectsError> {
        self.projects
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| not_found(id))
    }

    fn insert(&mut self, project: Project, top: bool) {
        if top {
            self.projects.insert(0, project);
        } else {
            self.projects.push(project);
        }
    }

    /// A user-created project at the top of the order.
    ///
    /// # Errors
    /// [`ProjectsError::Invalid`] for an empty or long name, or past [`MAX_PROJECTS`].
    pub fn create(
        &mut self,
        name: &str,
        main_folder: Option<&str>,
        now: u64,
    ) -> Result<Project, ProjectsError> {
        let name = clean_name(name)?;
        if self.projects.len() >= MAX_PROJECTS {
            return Err(ProjectsError::Invalid(format!(
                "at most {MAX_PROJECTS} projects"
            )));
        }
        let project = Project {
            id: new_id(now),
            name,
            main_folder: main_folder.map(normalize_folder),
            auto: false,
            created_ms: now,
        };
        self.insert(project.clone(), true);
        Ok(project)
    }

    /// Renames a project.
    ///
    /// # Errors
    /// [`ProjectsError::NotFound`]; [`ProjectsError::Invalid`] for an empty or long name.
    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), ProjectsError> {
        let name = clean_name(name)?;
        let at = self.index(id)?;
        self.projects[at].name = name;
        Ok(())
    }

    /// Deletes a project. Its sessions become "not in a project"; no session is removed. Returns how many moved.
    ///
    /// # Errors
    /// [`ProjectsError::NotFound`].
    pub fn delete(&mut self, id: &str) -> Result<usize, ProjectsError> {
        let at = self.index(id)?;
        self.projects.remove(at);
        let mut moved = 0;
        for project in self.members.values_mut() {
            if project.as_deref() == Some(id) {
                *project = None;
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Files `sessions` into `target` (`None`: out of any project). A session must be a member already or `known`
    /// (loaded or listed by Vibe). Returns how many changed.
    ///
    /// # Errors
    /// [`ProjectsError::NotFound`] for an unknown project or session (nothing changes);
    /// [`ProjectsError::Invalid`] for no ids or more than [`MAX_MOVE`].
    pub fn move_sessions(
        &mut self,
        sessions: &[String],
        target: Option<&str>,
        known: impl Fn(&str) -> bool,
    ) -> Result<usize, ProjectsError> {
        if sessions.is_empty() || sessions.len() > MAX_MOVE {
            return Err(ProjectsError::Invalid(format!(
                "move 1 to {MAX_MOVE} sessions"
            )));
        }
        if let Some(target) = target {
            self.index(target)?;
        }
        if let Some(unknown) = sessions
            .iter()
            .find(|s| !self.members.contains_key(s.as_str()) && !known(s))
        {
            return Err(ProjectsError::NotFound(format!(
                "Vibe Work does not know the session {unknown:.80}"
            )));
        }
        let mut moved = 0;
        for session in sessions {
            let now = target.map(str::to_owned);
            if self.members.get(session) != Some(&now) {
                self.members.insert(session.clone(), now);
                self.missing.remove(session);
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Moves project `id` before project `before` (`None`: to the end).
    ///
    /// # Errors
    /// [`ProjectsError::NotFound`] for either id.
    pub fn place(&mut self, id: &str, before: Option<&str>) -> Result<(), ProjectsError> {
        let from = self.index(id)?;
        if let Some(before) = before {
            self.index(before)?;
        }
        if before == Some(id) {
            return Ok(());
        }
        let project = self.projects.remove(from);
        let to = match before {
            Some(before) => self.index(before)?,
            None => self.projects.len(),
        };
        self.projects.insert(to, project);
        Ok(())
    }

    /// Sets or clears a project's main folder (the caller checked that the folder may be used).
    ///
    /// # Errors
    /// [`ProjectsError::NotFound`]; [`ProjectsError::Invalid`] for an empty folder.
    pub fn set_main_folder(&mut self, id: &str, folder: Option<&str>) -> Result<(), ProjectsError> {
        if folder.is_some_and(|f| f.trim().is_empty()) {
            return Err(ProjectsError::Invalid(
                "the main folder is empty".to_owned(),
            ));
        }
        let at = self.index(id)?;
        self.projects[at].main_folder = folder.map(normalize_folder);
        Ok(())
    }

    /// The folder rule: the first project in order whose main folder is `folder`, else a new auto project for it
    /// (at the top or the end). `None` when there is no folder or the store is full.
    fn by_folder(&mut self, folder: &str, top: bool, batch: &[String], now: u64) -> Option<String> {
        let folder = normalize_folder(folder);
        if folder.trim().is_empty() {
            return None;
        }
        if let Some(found) = self.projects.iter().find(|p| {
            p.main_folder
                .as_deref()
                .is_some_and(|m| same_folder(m, &folder))
        }) {
            return Some(found.id.clone());
        }
        if self.projects.len() >= MAX_PROJECTS {
            return None;
        }
        let taken: BTreeSet<String> = self.projects.iter().map(|p| p.name.clone()).collect();
        let folders: Vec<&str> = self
            .projects
            .iter()
            .filter_map(|p| p.main_folder.as_deref())
            .collect();
        let project = Project {
            id: new_id(now),
            name: auto_name(&folder, &taken, &folders, batch),
            main_folder: Some(folder),
            auto: true,
            created_ms: now,
        };
        let id = project.id.clone();
        self.insert(project, top);
        Some(id)
    }

    fn remember(&mut self, session: &str, project: Option<String>) -> bool {
        if self.members.len() >= MAX_MEMBERS {
            tracing::warn!("the projects store is full: a session is not remembered");
            return false;
        }
        self.members.insert(session.to_owned(), project);
        self.missing.remove(session);
        true
    }

    /// Files a session Vibe Work saw being created (the filing rules). A session already filed keeps
    /// its project. Returns true when it was filed now.
    pub fn file_new(&mut self, session: &NewSession<'_>, now: u64) -> bool {
        if session.id.is_empty() || self.members.contains_key(session.id) {
            return false;
        }
        let hinted = match &session.hint {
            Some(Hint::Project(id)) if self.project(id).is_some() => Some(Some(id.clone())),
            Some(Hint::Unfiled) => Some(None),
            _ => None,
        };
        let inherited = || session.source.and_then(|s| self.members.get(s)).cloned();
        let target = match hinted.or_else(inherited) {
            Some(target) => target,
            None => session
                .folder
                .and_then(|f| self.by_folder(f, true, &[], now)),
        };
        self.remember(session.id, target)
    }

    /// Files sessions seen for the first time in a listing (or resumed) by their folder; new auto projects go to
    /// the end. Already-filed sessions are left alone. Returns how many were filed.
    pub fn file_seen(&mut self, items: &[Seen], now: u64) -> usize {
        let fresh: Vec<&Seen> = items
            .iter()
            .filter(|s| !s.id.is_empty() && !self.members.contains_key(&s.id))
            .collect();
        // One entry per folder, whatever its spelling (the first spelling seen is kept).
        let mut batch: Vec<String> = Vec::new();
        for folder in fresh
            .iter()
            .filter_map(|s| s.folder.as_deref().map(normalize_folder))
        {
            if !batch.iter().any(|b| same_folder(b, &folder)) {
                batch.push(folder);
            }
        }
        batch.sort();
        let mut filed = 0;
        for item in fresh {
            let target = item
                .folder
                .as_deref()
                .and_then(|f| self.by_folder(f, false, &batch, now));
            if self.remember(&item.id, target) {
                filed += 1;
            }
        }
        filed
    }

    /// Forgets a session Vibe deleted. Returns true when it was a member.
    pub fn forget(&mut self, session: &str) -> bool {
        self.missing.remove(session);
        self.members.remove(session).is_some()
    }

    /// Reconciles with a **complete** listing (every page, archived included): files unseen sessions, marks members
    /// that are neither `listed` nor `loaded` as missing, and drops those first missed at least [`DROP_AFTER_MS`]
    /// before `now` (a member that shows up again loses its mark).
    pub fn sweep(&mut self, listed: &[Seen], loaded: &BTreeSet<String>, now: u64) -> SweepOutcome {
        let filed = self.file_seen(listed, now);
        let live: BTreeSet<&str> = listed
            .iter()
            .map(|s| s.id.as_str())
            .chain(loaded.iter().map(String::as_str))
            .collect();
        let gone: Vec<String> = self
            .members
            .keys()
            .filter(|id| !live.contains(id.as_str()))
            .cloned()
            .collect();
        let mut outcome = SweepOutcome {
            filed,
            ..SweepOutcome::default()
        };
        let mut still: BTreeMap<String, u64> = BTreeMap::new();
        for id in gone {
            match self.missing.get(&id) {
                Some(since) if now.saturating_sub(*since) >= DROP_AFTER_MS => {
                    self.members.remove(&id);
                    outcome.dropped += 1;
                }
                since => {
                    still.insert(id, since.copied().unwrap_or(now));
                    outcome.missing += 1;
                }
            }
        }
        self.missing = still;
        outcome
    }
}
