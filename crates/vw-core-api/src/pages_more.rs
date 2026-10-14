//! The Handbook Format 1.2 requests of the `pages` command: input answers, the app theme for the viewer,
//! the plan checklist's current plan, export as one file and the visual check; everything else goes on to the
//! stored-data requests (`pages_data`). Split out of `pages` to keep its dispatcher short.
//!
//! Depends on: `pages_answers`, `pages_plan`, `pages_share`, `pages_data`, vw-pages. Called by: `pages`.

use vw_pages::PagesService;

use crate::command::Origin;
use crate::core::Core;
use crate::error::CoreError;
use crate::event::CoreEvent;
use crate::pages::{core_error, path_of};
use crate::pages_view::{PagesRequest, PagesResult};

impl Core {
    /// Routes the Format 1.2 requests, then the stored-data ones.
    pub(crate) async fn pages_more(
        &self,
        service: &PagesService,
        origin: Origin,
        request: PagesRequest,
    ) -> Result<PagesResult, CoreError> {
        Ok(match request {
            request @ (PagesRequest::Answers { .. }
            | PagesRequest::SetAnswer { .. }
            | PagesRequest::ClearAnswer { .. }
            | PagesRequest::MarkAnswersSent { .. }) => self.pages_answers(service, request).await?,
            PagesRequest::Theme { theme } => {
                service
                    .set_theme(&theme)
                    .await
                    .map_err(|e| core_error(&e))?;
                PagesResult::Done {}
            }
            PagesRequest::CurrentPlan { session_id } => {
                self.current_plan(service, &session_id).await?
            }
            PagesRequest::ExportPage {
                session_id,
                path,
                destination,
            } => {
                self.export_page(service, origin, &session_id, &path, destination)
                    .await?
            }
            PagesRequest::VisualCheck { session_id, path } => {
                let project = self.project_of(&session_id)?;
                let report = service
                    .visual_check(&project, &path_of(&path)?)
                    .await
                    .map_err(|e| core_error(&e))?;
                self.shared.emit(CoreEvent::PagesChanged {
                    session_id: Some(session_id),
                });
                PagesResult::Visual { report }
            }
            other => self.pages_data(service, other).await?,
        })
    }
}
