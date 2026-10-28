//! The Handbook Format 1.2 requests of the `pages` command: input answers, the app theme for the viewer,
//! the plan checklist's current plan, export as one file and the visual check; everything else goes on to the
//! stored-data requests (`pages_data`); also the page menu's reads (`menuSection`, `menuCopy`). Split out of `pages`
//! to keep its dispatcher short.
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
                // The Browser's page menu falls back to the app's theme too (pages loaded from now on).
                if let Ok(preview) = crate::previews::hub(&self.shared) {
                    preview.service.set_app_theme(&theme);
                }
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
            PagesRequest::MenuSection {
                session_id,
                section,
                quote,
            } => {
                check_menu_target(origin, section.as_deref(), quote.as_deref())?;
                let project = self.project_of(&session_id)?;
                PagesResult::Picked {
                    picked: match section {
                        Some(section) => {
                            service
                                .menu_section_named(
                                    &session_id,
                                    &project,
                                    &section,
                                    quote.as_deref(),
                                )
                                .await
                        }
                        None => service.menu_section(&session_id, &project).await,
                    }
                    .map_err(|e| core_error(&e))?,
                }
            }
            PagesRequest::MenuCopy {
                session_id,
                section,
                quote,
            } => {
                check_menu_target(origin, section.as_deref(), quote.as_deref())?;
                PagesResult::Copied {
                    text: match section {
                        Some(section) => {
                            service
                                .menu_copy_named(&session_id, &section, quote.as_deref())
                                .await
                        }
                        None => service.menu_copy(&session_id).await,
                    }
                    .map_err(|e| core_error(&e))?,
                }
            }
            other => self.pages_data(service, other).await?,
        })
    }
}

fn check_menu_target(
    _origin: Origin,
    section: Option<&str>,
    quote: Option<&str>,
) -> Result<(), CoreError> {
    if section.is_some() || quote.is_some() {
        crate::preview_pick::section_name(section)?;
    }
    if quote.is_some_and(|q| q.is_empty() || q.chars().count() > vw_pages::MAX_COPY) {
        return Err(CoreError::new(
            crate::error::ErrorCode::InvalidParams,
            "quote must contain 1 to 100000 characters",
        ));
    }
    Ok(())
}
