//! The plan checklist's data in the core ("Plan checklist in the side panel"): `currentPlan` answers the
//! facts of the session project's current plan page without opening it (one source: the page FILE), and the reload
//! tick watches that file so the checklist updates while the agent works (`pagesChanged` for the session).
//!
//! Depends on: vw-pages (`PagesService::{current_plan, page}`), `pages` (the hub's plan watch, helpers). Called by:
//! `pages` (`currentPlan`, the tick), `pages::session_ended` (forgets the watch).
//! Invariant: only the plan file a `currentPlan` answer named is watched per session (one `stat` per tick each).

use std::sync::{Arc, PoisonError};

use vw_pages::PagesService;

use crate::core::{Core, Shared};
use crate::error::CoreError;
use crate::event::CoreEvent;
use crate::pages::{blocking, core_error};
use crate::pages_view::PagesResult;

impl Core {
    /// `currentPlan`: the current plan's facts (or none), and the watch on its file.
    pub(crate) async fn current_plan(
        &self,
        service: &PagesService,
        session_id: &str,
    ) -> Result<PagesResult, CoreError> {
        let project = self.project_of(session_id)?;
        let owned = service.clone();
        let found = blocking(move || {
            let Some(plan) = owned.current_plan(&project) else {
                return Ok(None);
            };
            let facts = owned.page(&project, &plan)?;
            let file = plan.under(&owned.pages_dir(&project));
            Ok::<_, vw_pages::PagesError>(Some((plan, facts, file)))
        })
        .await?
        .map_err(|e| core_error(&e))?;
        let Some((plan, facts, file)) = found else {
            if let Some(hub) = &self.shared.pages {
                hub.plans
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(session_id);
            }
            return Ok(PagesResult::CurrentPlan {
                path: None,
                facts: None,
            });
        };
        if let Some(hub) = &self.shared.pages {
            let seen = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
            hub.plans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(session_id.to_owned(), (file, seen));
        }
        Ok(PagesResult::CurrentPlan {
            path: Some(plan.to_string()),
            facts: Some(Box::new(facts)),
        })
    }
}

/// One reload tick: sessions whose watched plan file changed get `pagesChanged` (the checklist reads it again).
pub(crate) fn plan_tick(shared: &Arc<Shared>) {
    let Some(hub) = &shared.pages else { return };
    let changed: Vec<String> = {
        let mut plans = hub.plans.lock().unwrap_or_else(PoisonError::into_inner);
        plans
            .iter_mut()
            .filter_map(|(session, (file, seen))| {
                let now = std::fs::metadata(&*file).and_then(|m| m.modified()).ok();
                (now != *seen).then(|| {
                    *seen = now;
                    session.clone()
                })
            })
            .collect()
    };
    for session_id in changed {
        shared.emit(CoreEvent::PagesChanged {
            session_id: Some(session_id),
        });
    }
}
