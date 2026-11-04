//! Document-bound browser observation, frame merge and optional visual gap pass.
//! Called by reads and receipts; all output passes policy, bounding, prompt guard and current-mount checks.
use crate::{
    Browser, ReadOptions, guard,
    model::{PageModel, RefItem},
    observe::{self, PAGE_CHARS},
    refs::PAGE_CLOSED,
    scripts,
};
const FIND_TEXT: usize = 20_000;

/// Caps an already priority-ordered list, reserving half (rounded up) for available main refs.
/// Full observations and focused output both use this, before any smaller byte-budget cap.
pub(crate) fn reserve_main_refs(refs: &mut Vec<RefItem>, max_refs: usize) {
    let mut reserved = max_refs
        .div_ceil(2)
        .min(refs.iter().filter(|item| item.frame != Some(true)).count());
    let mut remaining = max_refs;
    refs.retain(|item| {
        let main = item.frame != Some(true);
        if remaining == 0 || (!main && remaining <= reserved) {
            return false;
        }
        remaining -= 1;
        if main {
            reserved = reserved.saturating_sub(1);
        }
        true
    });
}

impl Browser {
    /// Observes the page, retaining earlier refs in this document and replacing the receipt baseline.
    ///
    /// # Errors
    /// A plain-text reason (nothing open, the page did not answer).
    pub async fn observe(
        &self,
        session_id: &str,
        options: ReadOptions,
    ) -> Result<(PageModel, guard::Verdict, String), String> {
        self.observe_pass(session_id, options, None, true, true)
            .await
    }

    pub(crate) async fn observe_dom(&self, session_id: &str) -> Result<PageModel, String> {
        self.observe_pass(
            session_id,
            ReadOptions {
                max_refs: crate::model::MAX_REFS,
                ..ReadOptions::default()
            },
            None,
            false,
            true,
        )
        .await
        .map(|(model, _, _)| model)
    }

    /// Observes the page's frames again without replacing the session's latest observation: a coordinate click
    /// keeps the refs and geometry of the read its coordinates came from. With `node`, the refs are only that
    /// observed node's (in each frame), so a dense page's ref cap never leaves it out; the frames are observed whole.
    pub(crate) async fn observe_unrecorded(
        &self,
        session_id: &str,
        node: Option<u64>,
    ) -> Result<PageModel, String> {
        self.observe_pass(
            session_id,
            ReadOptions {
                max_refs: crate::model::MAX_REFS,
                ..ReadOptions::default()
            },
            node,
            false,
            false,
        )
        .await
        .map(|(model, _, _)| model)
    }

    /// Numbers the refs for the labels drawn on a read's screenshot (1, 2, ... in ref order). Only a read that
    /// attaches a screenshot numbers them, so other answers do not carry labels the agent cannot see.
    pub(crate) fn label_refs(model: &mut PageModel) {
        for (index, item) in model.refs.iter_mut().enumerate() {
            item.label = Some((index + 1).to_string());
        }
    }

    async fn observe_pass(
        &self,
        session_id: &str,
        options: ReadOptions,
        node: Option<u64>,
        pixels: bool,
        record: bool,
    ) -> Result<(PageModel, guard::Verdict, String), String> {
        let page = self.page(session_id).await?;
        let mount = self
            .service
            .page_mount(session_id)
            .ok_or_else(|| PAGE_CLOSED.to_owned())?;
        let offset = options.text_page.unwrap_or(0).saturating_mul(PAGE_CHARS);
        let (max_refs, max_text, offset) = if options.find.is_some() || node.is_some() {
            (crate::model::MAX_REFS, FIND_TEXT, offset)
        } else {
            (
                options.max_refs.clamp(1, crate::model::MAX_REFS),
                PAGE_CHARS,
                offset,
            )
        };
        let script = scripts::observe_focused(
            &page,
            max_refs,
            max_text,
            offset,
            options.find.as_deref(),
            node,
        );
        let loads_before = self.service.load_state(session_id).map(|l| l.loads);
        let mut model: PageModel = match self
            .observe_frames(session_id, &mount, &page, &script)
            .await?
        {
            Some(model) => model,
            None => self.run_in_mount(session_id, script, Some(&mount)).await?,
        };
        // Authorization binds to the document that answered: the same mount, a page it may show.
        self.service
            .reading_allowed(session_id, &page, &model.url)?;
        if self.service.page_mount(session_id).as_ref() != Some(&mount) {
            return Err(PAGE_CLOSED.to_owned());
        }
        if pixels {
            self.visual_refs(session_id, &mut model).await;
        }
        self.service
            .reading_allowed(session_id, &page, &model.url)?;
        if self.service.page_mount(session_id).as_ref() != Some(&mount) {
            return Err(PAGE_CLOSED.to_owned());
        }
        // Keep priority order (including page-covering consent dialogs), but reserve half the
        // slots for main-document candidates. Select before bound's absolute 200-ref cap too.
        model
            .refs
            .sort_by_key(|item| std::cmp::Reverse(item.priority));
        model.refs.retain(|item| crate::model::is_ref(&item.id));
        reserve_main_refs(&mut model.refs, max_refs);
        model.bound(max_text);
        let verdict = observe::verdict(&model);
        let mut seen = observe::seen(&model, mount);
        // which document this observation saw (its load count), for the fill's "current observation". A
        // load that finished while the script ran leaves it unknown (a fill then asks for a fresh read).
        let loads_after = self.service.load_state(session_id).map(|l| l.loads);
        seen.loads = loads_before.filter(|_| loads_before == loads_after);
        let snapshot = seen.snapshot.clone();
        if record {
            self.sessions.set_seen(session_id, seen);
        }
        Ok((model, verdict, snapshot))
    }
}
