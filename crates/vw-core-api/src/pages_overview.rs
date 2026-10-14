//! The handbook reads of the core: the session project's live hub (the Project tab), the Projects
//! overview (one card per known project, heads only) and one known project's hub without a session (the overview's
//! health line and detail pane). Everything here reads; nothing is written into a repository.
//!
//! Depends on: vw-pages (`PagesService::{hub, card, settings}`), `pages` (errors, blocking helper, names),
//! `plan_profile` (the known projects), `projects_filing` (main folders). Called by: `pages` (`Core::pages`).
//! Invariants:
//! - `projectHub` accepts only a folder of the known set (`plan_profile::projects` plus the main folders of the
//!   user's projects), compared as written and canonicalized; any other path is refused with `notFound`
//!   before a file is touched.
//! - The overview has one card per distinct main folder of the user's projects (projects are the user's
//!   groups; the UI joins cards to projects by main folder).
//! - File work runs on the blocking pool; the overview reads only manifest heads and active plan heads.

use std::path::{Path, PathBuf};
use std::time::Instant;

use vw_pages::PagesService;

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::pages::{blocking, name_of};
use crate::pages_view::{PagesResult, ProjectHub};

impl Core {
    /// `hub`: the session project's live hub with its settings, comment counts and viewer.
    pub(crate) async fn pages_hub(
        &self,
        service: &PagesService,
        session_id: &str,
    ) -> Result<PagesResult, CoreError> {
        let project = self.project_of(session_id)?;
        crate::plan_profile::note_project(&self.shared, &project.to_string_lossy());
        let settings = service.settings(&project);
        let (owned, folder) = (service.clone(), project.clone());
        let (hub, counts, visual) = blocking(move || {
            let hub = owned.hub(&folder).map(Box::new);
            let counts = owned.comments().counts(&folder).into_iter().collect();
            // The current visual check reports (cached, never a render here).
            let pages = hub
                .as_deref()
                .map(crate::pages_look::pages_to_check)
                .unwrap_or_default();
            let visual = owned.visual_reports(&folder, &pages);
            (hub, counts, visual)
        })
        .await?;
        if let Some(hub) = &hub {
            // The page shown first, then the newest pages: checked in the background before the user opens them.
            let mut queue: Vec<String> = service
                .view(session_id)
                .and_then(|v| v.path)
                .into_iter()
                .collect();
            queue.extend(
                crate::pages_look::pages_to_check(hub)
                    .into_iter()
                    .filter(|p| !visual.contains_key(p)),
            );
            crate::pages_look::queue_checks(&self.shared, &project, queue);
        }
        Ok(PagesResult::Hub {
            name: name_of(&project),
            project: project.display().to_string(),
            settings,
            hub,
            comment_counts: counts,
            viewer: service.view(session_id),
            visual,
        })
    }

    /// `overview`: a card per main folder of the user's projects, sorted by name.
    pub(crate) async fn overview(&self, service: &PagesService) -> Result<PagesResult, CoreError> {
        let projects = crate::projects_filing::main_folders(&self.shared);
        let owned = service.clone();
        let (projects, elapsed_ms) = blocking(move || {
            let started = Instant::now();
            let mut cards: Vec<_> = projects
                .into_iter()
                .map(|p| owned.card(Path::new(&p)))
                .collect();
            cards.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.project.cmp(&b.project)));
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            (cards, elapsed)
        })
        .await?;
        Ok(PagesResult::Overview {
            projects,
            elapsed_ms,
        })
    }

    /// `projectHub`: the live hub of a known project.
    pub(crate) async fn project_hub(
        &self,
        service: &PagesService,
        project: &str,
    ) -> Result<PagesResult, CoreError> {
        let mut known = crate::plan_profile::projects(&self.shared);
        known.extend(crate::projects_filing::main_folders(&self.shared));
        let folder = known_project(&known, project).ok_or_else(|| {
            CoreError::new(
                ErrorCode::NotFound,
                "this folder is not one of Vibe Work's projects (only folders of known sessions)",
            )
        })?;
        let owned = service.clone();
        let hub = blocking(move || {
            let hub = owned.hub(&folder);
            ProjectHub {
                name: name_of(&folder),
                project: folder.display().to_string(),
                hub,
            }
        })
        .await?;
        Ok(PagesResult::ProjectHub {
            project: Box::new(hub),
        })
    }
}

/// The known project `asked` names: equal as written, or equal after resolving both (a symlinked folder).
fn known_project(known: &[String], asked: &str) -> Option<PathBuf> {
    if let Some(found) = known.iter().find(|k| k.as_str() == asked) {
        return Some(PathBuf::from(found));
    }
    let resolved = std::fs::canonicalize(asked).ok()?;
    known
        .iter()
        .find(|k| std::fs::canonicalize(k).is_ok_and(|c| c == resolved))
        .map(PathBuf::from)
}
