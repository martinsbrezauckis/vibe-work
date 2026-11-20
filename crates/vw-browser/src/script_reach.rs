//! What a page script could reach is checked again when its result is collected, before anything it returned (or
//! threw) leaves the browser. The check before dispatch sees only the page as it was; the script can load another page
//! of the same site into a frame (or the page itself), read it, and navigate, empty or remove it before it returns. So
//! the result is withheld ([`RESULT_WITHHELD`]) when:
//! - any frame of the page, the top one included, was allowed to load a document of the script's own site during
//!   the run (the engine's navigation hooks, vw-preview `document_journal`), whatever that frame holds at collection
//!   or whether it still exists. With no known site, every such document counts. Documents the script wrote itself
//!   (`about:`, `data:`, `blob:`, `javascript:`) and documents of other sites (which the script cannot read) do not;
//! - the page asked for a new window during the run, whatever its address (every engine denies the native window and
//!   the host opens the address as a tab of its own, with no opener, but an opener could steer it);
//! - on `WebView2`, where a cached or restored frame document may not pass the request barrier, the frame registry
//!   changed during the run (a frame was attached, navigated or removed);
//! - a frame of the page holds a protected field at collection (the shared rule, every frame counted), or a check
//!   cannot run (counted as one protected field).
//!
//! The script did run, so the act failed (effects may have happened). The tab then loads the address it had before
//! the run again (a reload when that is refused), so a value the script copied into the page's own document or
//! address is not left for a page read or a capture, and its site is remembered (`protected_origins`): later raw
//! actions there are refused before dispatch.
use serde_json::Value;

use crate::page_scripts::Request;
use crate::protected_origins::{RESULT_WITHHELD, Why};
use crate::{ActError, Actor, Browser};

impl Browser {
    /// Runs a page script and withholds its result when it reached a protected field.
    pub(crate) async fn page_script_checked(
        &self,
        session: &str,
        request: &Request,
        by: Actor,
        nav: &dyn crate::Navigate,
    ) -> Result<Value, ActError> {
        let site = self.page_site(session);
        let address = self.service.view(session).and_then(|view| view.url);
        // No page: nothing can run (the page script refuses it).
        let Some(mount) = self.service.page_mount(session) else {
            return self.page_script(session, request, by).await;
        };
        let registry = self.frame_registry(&mount).await;
        let mark = self.service.document_mark();
        let result = self.page_script(session, request, by).await;
        let mut reached = self
            .reach_check(session, &mount, site.as_deref(), mark)
            .await;
        if reached.is_ok() && (registry.is_none() || self.frame_registry(&mount).await != registry)
        {
            reached = Err(1);
        }
        let Err(count) = reached else {
            return result;
        };
        if let Some(site) = &site {
            self.remember_site_of(site, Why::Reached);
        }
        self.remember_page_site(session, Why::Reached);
        // Best effort: a page that cannot load again keeps its site off all the same.
        let reopened = match &address {
            Some(address) => nav.open(session, address).await.is_ok(),
            None => false,
        };
        if reopened
            || self
                .service
                .history(session, vw_preview::HistoryStep::Reload)
                .await
                .is_ok()
        {
            self.wait_loaded(session).await;
        }
        let mut error = crate::driver::failed(RESULT_WITHHELD);
        error.protected_values_withheld = Some(count);
        Err(error)
    }

    /// `Err(protected fields)` when the run reached one or cannot be proved clean (counted as one).
    async fn reach_check(
        &self,
        session: &str,
        mount: &vw_preview::MountId,
        site: Option<&str>,
        mark: u64,
    ) -> Result<(), u64> {
        // A script that navigated the page: the new document is checked once it has loaded.
        self.wait_loaded(session).await;
        if self.service.page_mount(session).as_ref() != Some(mount) {
            return Err(1);
        }
        match self.protected_field_count(session).await {
            Ok(0) => {}
            Ok(count) => return Err(count),
            Err(_) => return Err(1),
        }
        let loaded = self.service.documents_since(mount, mark).ok_or(1_u64)?;
        // A page without an origin of its own (`file:`, a sandboxed document) is not kept apart from other
        // documents by origin: every document counts for it.
        let any_site = site
            .is_none_or(|site| url::Url::parse(site).map_or(true, |url| !url.origin().is_tuple()));
        let reached = loaded.iter().any(|row| {
            row.window
                || (!matches!(row.url.scheme(), "about" | "data" | "blob" | "javascript")
                    && (any_site
                        || crate::protected_origins::site_of(row.url.as_str()).as_deref() == site))
        });
        if reached { Err(1) } else { Ok(()) }
    }

    /// `WebView2`: the frame registry's generation, which moves when a frame is attached, navigated or removed;
    /// `None` when it cannot be read. Elsewhere the navigation hooks see every frame document, and this is constant.
    async fn frame_registry(&self, mount: &vw_preview::MountId) -> Option<u64> {
        if !cfg!(windows) {
            return Some(0);
        }
        let answers = self
            .service
            .evaluate_frames_on_mount(mount, None, "null".into(), None)
            .await
            .ok()??;
        let first = answers.first()?.generation?;
        answers
            .iter()
            .all(|answer| answer.generation == Some(first))
            .then_some(first)
    }
}
