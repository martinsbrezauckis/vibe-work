//! Project Pages in the core, the stored-data half (split out of `pages`): comments (add, delete, mark
//! sent), settings, and one page's facts with its comments.
//!
//! Depends on: vw-pages, `pages` (errors, blocking helper, the session's project), `plan_profile` (a moved pages
//! folder changes the profile). Called by: `pages` (`Core::pages`).
//! Invariant: a comment's section must exist in the page FILE when it is added (never trusted from the client).

use std::path::Path;

use vw_pages::{PagePath, PagesError, PagesService};

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::pages::{blocking, core_error, path_of};
use crate::pages_view::{PagesRequest, PagesResult};

impl Core {
    /// The requests about stored data: comments and settings.
    pub(crate) async fn pages_data(
        &self,
        service: &PagesService,
        request: PagesRequest,
    ) -> Result<PagesResult, CoreError> {
        let shared = &self.shared;
        let err = |e: PagesError| core_error(&e);
        Ok(match request {
            PagesRequest::AddComment {
                session_id,
                path,
                section,
                text,
                quote,
            } => {
                let project = self.project_of(&session_id)?;
                let page = path_of(&path)?;
                let comment = add_comment(service, &project, page, &section, &text, quote).await?;
                shared.emit(CoreEvent::PagesChanged {
                    session_id: Some(session_id),
                });
                PagesResult::Comment { comment }
            }
            PagesRequest::DeleteComment { session_id, id } => {
                let project = self.project_of(&session_id)?;
                let owned = service.clone();
                blocking(move || owned.comments().delete(&project, &id))
                    .await?
                    .map_err(|e| err(e.into()))?;
                shared.emit(CoreEvent::PagesChanged {
                    session_id: Some(session_id),
                });
                PagesResult::Done {}
            }
            PagesRequest::MarkSent { session_id, ids } => {
                let project = self.project_of(&session_id)?;
                let owned = service.clone();
                let comments = blocking(move || owned.comments().mark_sent(&project, &ids))
                    .await?
                    .map_err(|e| err(e.into()))?;
                shared.emit(CoreEvent::PagesChanged {
                    session_id: Some(session_id),
                });
                PagesResult::Comments { comments }
            }
            PagesRequest::Settings { session_id } => PagesResult::Settings {
                settings: service.settings(&self.project_of(&session_id)?),
            },
            PagesRequest::SetSettings {
                session_id,
                settings,
            } => {
                let project = self.project_of(&session_id)?;
                let saved = service
                    .set_settings(&project, &settings)
                    .await
                    .map_err(err)?;
                crate::plan_profile::refresh(shared);
                PagesResult::Settings { settings: saved }
            }
            other => {
                return Err(CoreError::new(
                    ErrorCode::Internal,
                    format!(
                        "pages request {:?} has no handler",
                        std::mem::discriminant(&other)
                    ),
                ));
            }
        })
    }
}

pub(crate) async fn page_result(
    service: &PagesService,
    session_id: &str,
    project: &Path,
    page: PagePath,
) -> Result<PagesResult, CoreError> {
    let (owned, folder, path) = (service.clone(), project.to_path_buf(), page.clone());
    let (facts, attached, detached) = blocking(move || {
        let facts = owned.page(&folder, &path)?;
        let ids: Vec<String> = facts.sections.iter().map(|s| s.id.clone()).collect();
        let (attached, detached) = owned.comments().list(&folder, &path, Some(&ids));
        Ok::<_, PagesError>((facts, attached, detached))
    })
    .await?
    .map_err(|e| core_error(&e))?;
    Ok(PagesResult::Page {
        path: page.to_string(),
        facts: Box::new(facts),
        attached,
        detached,
        viewer: service.view(session_id),
    })
}

async fn add_comment(
    service: &PagesService,
    project: &Path,
    page: PagePath,
    section: &str,
    text: &str,
    quote: Option<String>,
) -> Result<vw_pages::Comment, CoreError> {
    let (owned, folder, section, text) = (
        service.clone(),
        project.to_path_buf(),
        section.to_owned(),
        text.to_owned(),
    );
    blocking(move || {
        let facts = owned.page(&folder, &page)?;
        let label = facts
            .section_label(&section)
            .ok_or_else(|| PagesError::Other(format!("the page has no section '{section}'")))?
            .to_owned();
        owned
            .comments()
            .add(&folder, &page, (&section, &label), &text, quote.as_deref())
            .map_err(PagesError::from)
    })
    .await?
    .map_err(|e| match e {
        PagesError::Other(text) => CoreError::new(ErrorCode::InvalidParams, text),
        other => core_error(&other),
    })
}
