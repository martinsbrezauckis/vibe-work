//! The service side of page inputs (Handbook Format 1.2 §6.6): reads a page's inputs from its FILE and joins
//! them with the stored answers; sets, clears and marks answers sent, always checked against the file. Blocking file
//! reads: callers run these on a blocking task.
//!
//! Depends on: `answers`, `projects` (page facts, hub), `service`. Called by: vw-core-api (the `pages` answer
//! requests, the agent's `handbook_answers` tool).

use std::path::Path;

use crate::answers::{AnswerValue, PageAnswers, page_answers};
use crate::config::PagesError;
use crate::paths::PagePath;
use crate::service::PagesService;

impl PagesService {
    /// The inputs of `page` with the user's answers (an answer whose input is gone is detached).
    ///
    /// # Errors
    /// [`PagesError::NoPage`] when the page does not exist and has no stored answers either.
    pub fn answers_of(&self, project: &Path, page: &PagePath) -> Result<PageAnswers, PagesError> {
        let stored = self.answers.list(project, Some(page));
        match self.page(project, page) {
            Ok(facts) => Ok(page_answers(page, Some(&facts), stored)),
            Err(error) if stored.is_empty() => Err(error),
            Err(_) => Ok(page_answers(page, None, stored)),
        }
    }

    /// Every page of the project that has inputs or stored answers, in hub order (pages without either are left
    /// out).
    #[must_use]
    pub fn all_answers(&self, project: &Path) -> Vec<PageAnswers> {
        let mut pages: Vec<String> = self
            .hub(project)
            .map(|hub| {
                hub.pages()
                    .filter(|p| p.inputs > 0)
                    .map(|p| p.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        for page in self.answers.pages(project) {
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
        pages
            .iter()
            .filter_map(|p| PagePath::parse(p).ok())
            .filter_map(|p| self.answers_of(project, &p).ok())
            .collect()
    }

    /// Sets the answer to `input` of `page`, checked against the page file. Returns the page's answers.
    ///
    /// # Errors
    /// [`PagesError::NoPage`]; [`PagesError::Answer`] for a value that does not fit or a write failure.
    pub fn set_answer(
        &self,
        project: &Path,
        page: &PagePath,
        input: &str,
        value: &AnswerValue,
    ) -> Result<PageAnswers, PagesError> {
        let facts = self.page(project, page)?;
        self.answers.set(project, page, &facts, input, value)?;
        self.answer_revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.answers_of(project, page)
    }

    /// Removes the answer to `input` of `page` (also a detached one). Returns the page's answers.
    ///
    /// # Errors
    /// [`PagesError::Answer`] for a write failure; [`PagesError::NoPage`] when nothing is left to show.
    pub fn clear_answer(
        &self,
        project: &Path,
        page: &PagePath,
        input: &str,
    ) -> Result<PageAnswers, PagesError> {
        self.answers.clear(project, page, input)?;
        self.answer_revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.answers_of(project, page)
    }

    /// Marks every answer of `page` as put into the composer. Returns the page's answers.
    ///
    /// # Errors
    /// [`PagesError::Answer`] when the page has no answers or the write fails.
    pub fn mark_answers_sent(
        &self,
        project: &Path,
        page: &PagePath,
    ) -> Result<PageAnswers, PagesError> {
        self.answers.mark_sent(project, page)?;
        self.answer_revision
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.answers_of(project, page)
    }
}
