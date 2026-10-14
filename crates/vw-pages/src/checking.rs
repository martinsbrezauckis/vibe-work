//! The service side of the visual check: runs the checker on a page of a project's handbook and reads the
//! cached reports for the hub's badges.
//!
//! Depends on: `checker`, `service` (page servers), `projects` (folders). Called by: vw-core-api (`visualCheck`,
//! the hub answer, the agent's `handbook_visual_check`, background checks).

use std::collections::BTreeMap;
use std::path::Path;

use vw_preview::EngineError;

use crate::checker::VisualReport;
use crate::config::PagesError;
use crate::paths::PagePath;
use crate::service::PagesService;

impl PagesService {
    /// True when this service has a visual checker.
    #[must_use]
    pub fn can_check(&self) -> bool {
        self.checker.is_some()
    }

    /// Checks one page of `project`'s handbook (cached while the page and stylesheet are unchanged).
    ///
    /// # Errors
    /// [`PagesError::NoPagesFolder`], [`PagesError::NoPage`]; `Engine(Unsupported)` without a checker.
    pub async fn visual_check(
        &self,
        project: &Path,
        page: &PagePath,
    ) -> Result<VisualReport, PagesError> {
        let checker = self.checker.clone().ok_or_else(|| {
            PagesError::Engine(EngineError::Unsupported(
                "the visual check is not available in this build (no page renderer): skip it, do not retry; \
                 handbook_check still applies"
                    .to_owned(),
            ))
        })?;
        let server = self.server(project).await?;
        if !page.under(server.root()).is_file() {
            return Err(PagesError::NoPage(page.to_string()));
        }
        checker.check(&server, page).await
    }

    /// The current cached reports of `project`'s pages (blocking: hashes the files); pages never checked or changed
    /// since are left out.
    #[must_use]
    pub fn visual_reports(
        &self,
        project: &Path,
        pages: &[String],
    ) -> BTreeMap<String, VisualReport> {
        let Some(checker) = &self.checker else {
            return BTreeMap::new();
        };
        let root = self.pages_dir(project);
        pages
            .iter()
            .filter_map(|p| PagePath::parse(p).ok())
            .filter_map(|p| checker.cached(&root, &p).map(|r| (p.to_string(), r)))
            .collect()
    }

    /// Closes the checker's hidden webview (quit).
    pub async fn shutdown_checker(&self) {
        if let Some(checker) = &self.checker {
            checker.shutdown().await;
        }
    }
}
