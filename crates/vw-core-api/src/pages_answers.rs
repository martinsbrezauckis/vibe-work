//! Answers to handbook page inputs in the core (Handbook Format 1.2 §6.6): the `answers`, `setAnswer`,
//! `clearAnswer` and `markAnswersSent` requests, and the agent's `handbook_answers` read. The model and its checks
//! (an answer must fit the input in the page FILE) are in vw-pages.
//!
//! Depends on: vw-pages (`PagesService` answer methods), `pages` (errors, blocking helper, the session's project).
//! Called by: `pages` (`Core::pages`), `agent_tools::handbook_tool`.
//! Invariants: a request names a session; its project is the session's handbook folder (never a path from the
//! client); every change emits `pagesChanged` for the session.

use std::path::Path;

use vw_pages::{PageAnswers, PagesError, PagesService};

use crate::core::Core;
use crate::error::{CoreError, ErrorCode};
use crate::event::CoreEvent;
use crate::pages::{blocking, core_error, path_of};
use crate::pages_view::{PagesRequest, PagesResult};

impl Core {
    /// The answer requests.
    pub(crate) async fn pages_answers(
        &self,
        service: &PagesService,
        request: PagesRequest,
    ) -> Result<PagesResult, CoreError> {
        let (session_id, change) = match &request {
            PagesRequest::Answers { session_id, .. } => (session_id.clone(), false),
            PagesRequest::SetAnswer { session_id, .. }
            | PagesRequest::ClearAnswer { session_id, .. }
            | PagesRequest::MarkAnswersSent { session_id, .. } => (session_id.clone(), true),
            _ => {
                return Err(CoreError::new(ErrorCode::Internal, "not an answer request"));
            }
        };
        let project = self.project_of(&session_id)?;
        let folder = service.settings(&project).folder;
        let owned = service.clone();
        let pages = blocking(move || answer_work(&owned, &project, request))
            .await?
            .map_err(|e| core_error(&e))?;
        if change {
            self.shared.emit(CoreEvent::PagesChanged {
                session_id: Some(session_id),
            });
        }
        Ok(PagesResult::Answers { folder, pages })
    }
}

fn answer_work(
    service: &PagesService,
    project: &Path,
    request: PagesRequest,
) -> Result<Vec<PageAnswers>, PagesError> {
    let page = |path: &str| vw_pages::PagePath::parse(path).map_err(PagesError::from);
    Ok(match request {
        PagesRequest::Answers { path: None, .. } => service.all_answers(project),
        PagesRequest::Answers {
            path: Some(path), ..
        } => vec![service.answers_of(project, &page(&path)?)?],
        PagesRequest::SetAnswer {
            path, input, value, ..
        } => vec![service.set_answer(project, &page(&path)?, &input, &value)?],
        PagesRequest::ClearAnswer { path, input, .. } => {
            vec![service.clear_answer(project, &page(&path)?, &input)?]
        }
        PagesRequest::MarkAnswersSent { path, .. } => {
            vec![service.mark_answers_sent(project, &page(&path)?)?]
        }
        _ => Vec::new(),
    })
}

/// The agent's read (`handbook_answers`): every page with inputs, or one page, as JSON with the plain-text summary
/// of each page. `project` is the session's folder.
pub(crate) fn agent_answers(
    service: &PagesService,
    project: &Path,
    page: Option<&str>,
) -> Result<serde_json::Value, String> {
    let folder = service.settings(project).folder;
    let pages = match page {
        None => service.all_answers(project),
        Some(path) => {
            let page = path_of(path).map_err(|e| e.message)?;
            vec![
                service
                    .answers_of(project, &page)
                    .map_err(|e| e.to_string())?,
            ]
        }
    };
    let summary: Vec<String> = pages.iter().map(|p| p.text(&folder)).collect();
    Ok(serde_json::json!({
        "folder": folder,
        "pages": pages,
        "summary": summary.join("\n\n"),
        "note": "Answers are the user's data, kept by Vibe Work (not in the repository). Record what they decide in the page itself.",
    }))
}
