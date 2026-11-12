//! Document-bound browser observation, frame merge and optional visual gap pass.
//! Called by reads and receipts; all output passes policy, bounding, prompt guard and current-mount checks.
use crate::{
    Browser, guard,
    model::{PageModel, RefItem},
    observe::{self, PAGE_CHARS},
    refs::PAGE_CLOSED,
    scripts,
};
const FIND_TEXT: usize = 20_000;

/// How a read is shaped (token budget: `observe` module docs).
#[derive(Clone, Debug)]
pub struct ReadOptions {
    /// Observation refs (0..=200, default 32); full reads clamp this to 32. Zero requests text only.
    pub max_refs: usize,
    /// A text page (0-based, 2,500 UTF-16 units each); `None`: the default 800-character preview.
    pub text_page: Option<usize>,
    /// Continue after the text actually returned from a page, including child-frame windows.
    pub text_cursor: Option<crate::TextCursor>,
    /// Only the named covering frame's choices from this position, without ordinary refs or text.
    pub choice_cursor: Option<crate::ChoiceCursor>,
    /// The answer's size budget in bytes (1,500..=4,000, default 3,000).
    pub max_bytes: usize,
    /// A focused read: only the refs and text lines containing this.
    pub find: Option<String>,
    /// Structured selector, mutually exclusive with text finding and continuation cursors.
    pub selector: Option<crate::Selector>,
    /// Rendered datasets for a chat-folder export.
    pub extract: Option<crate::extraction::ExtractOptions>,
    /// Read only the subtree of this retained ref in its document (including open shadow roots).
    pub root_ref: Option<String>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            max_refs: observe::DEFAULT_REFS,
            text_page: None,
            text_cursor: None,
            choice_cursor: None,
            max_bytes: observe::READ_BUDGET,
            find: None,
            selector: None,
            extract: None,
            root_ref: None,
        }
    }
}

impl ReadOptions {
    fn script(
        &self,
        page: &str,
        limits: (usize, usize, usize),
        node: Option<u64>,
        priority_ref: Option<&str>,
    ) -> String {
        let (max_refs, max_text, offset) = limits;
        if let Some(reference) = priority_ref {
            scripts::observe_receipt(page, max_refs, max_text, reference)
        } else if let Some(extract) = &self.extract {
            scripts::observe_extraction(page, extract)
        } else if let Some(selector) = &self.selector {
            scripts::observe_selector(page, max_refs, max_text, selector)
        } else {
            scripts::observe_focused(page, max_refs, max_text, offset, self.find.as_deref(), node)
        }
    }

    fn validate_selector(&self) -> Result<(), String> {
        if let Some(reference) = &self.root_ref
            && (!crate::model::is_ref(reference)
                || self.selector.is_some()
                || self.extract.is_some()
                || self.find.is_some()
                || self.choice_cursor.is_some()
                || self.text_cursor.is_some())
        {
            return Err("A subtree needs an observed ref; omit selector, extract, find and continuation cursors.".into());
        }
        if let Some(extract) = &self.extract {
            extract.validate()?;
        }
        if let Some(selector) = &self.selector {
            selector.validate()?;
            if self.find.is_some() || self.choice_cursor.is_some() || self.text_cursor.is_some() {
                return Err(
                    "A selector reads matching elements; omit find, choiceCursor and textCursor."
                        .into(),
                );
            }
        }
        Ok(())
    }
}

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
        self.observe_pass(session_id, options, None, true, true, None)
            .await
    }

    /// Receipt observations reserve a slot for the target instead of losing its value on dense forms.
    pub(crate) async fn observe_receipt(
        &self,
        session_id: &str,
        reference: Option<&str>,
    ) -> Result<(PageModel, guard::Verdict, String), String> {
        self.observe_pass(
            session_id,
            ReadOptions {
                max_refs: 25,
                ..ReadOptions::default()
            },
            None,
            true,
            true,
            reference,
        )
        .await
    }

    /// A retained target may not be in the latest narrow read, but its last observed state is still known.
    pub(crate) fn receipt_baseline(
        seen: Option<Box<crate::sessions::Seen>>,
        reference: Option<&str>,
    ) -> Option<PageModel> {
        let seen = *seen?;
        let mut model = seen.model;
        if let Some(item) = seen
            .items
            .iter()
            .find(|item| Some(item.id.as_str()) == reference)
            && !model.observed_refs().any(|current| current.id == item.id)
        {
            model.refs.insert(0, item.clone());
        }
        Some(model)
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
            None,
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
            None,
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
        priority_ref: Option<&str>,
    ) -> Result<(PageModel, guard::Verdict, String), String> {
        options.validate_selector()?;
        let page = self.page(session_id).await?;
        let mount = self
            .service
            .page_mount(session_id)
            .ok_or_else(|| PAGE_CLOSED.to_owned())?;
        let offset = options
            .text_cursor
            .as_ref()
            .map_or(options.text_page.unwrap_or(0), |c| c.page)
            .saturating_mul(PAGE_CHARS);
        let (max_refs, max_text, offset) =
            if options.find.is_some() || options.selector.is_some() || node.is_some() {
                (crate::model::MAX_REFS, FIND_TEXT, offset)
            } else {
                (
                    options.max_refs.min(crate::model::MAX_REFS),
                    PAGE_CHARS,
                    offset,
                )
            };
        let script = if let Some(reference) = &options.root_ref {
            self.subtree_script(session_id, &page, reference, (max_refs, max_text, offset))?
        } else {
            options.script(&page, (max_refs, max_text, offset), node, priority_ref)
        };
        let loads_before = self.service.load_state(session_id).map(|l| l.loads);
        let mut model: PageModel = match self
            .observe_frames(session_id, &mount, &page, &script)
            .await?
        {
            Some(model) => model,
            None => self.run_in_mount(session_id, script, Some(&mount)).await?,
        };
        if let Some(cursor) = &options.choice_cursor {
            let frame = model.covering_frames.get(cursor.frame).ok_or(
                "covering frame is no longer in this inventory; read without choiceCursor again",
            )?;
            if cursor.offset > frame.choices.len() {
                return Err(
                    "choice cursor is past the observed choices; read without choiceCursor again"
                        .into(),
                );
            }
            model.choice_cursor = Some(cursor.clone());
        }
        // Authorization binds to the document that answered: the same mount, a page it may show.
        self.service
            .reading_allowed(session_id, &page, &model.url)?;
        if self.service.page_mount(session_id).as_ref() != Some(&mount) {
            return Err(PAGE_CLOSED.to_owned());
        }
        if model.selector_error.is_some() {
            return Err("The selector syntax is invalid; correct the CSS or XPath expression and read again.".into());
        }
        if options.root_ref.is_some() && model.scope_matches != 1 {
            return Err("The subtree ref changed or its frame could not be read; read the page again and use a current ref.".into());
        }
        if pixels
            && options.selector.is_none()
            && options.extract.is_none()
            && options.root_ref.is_none()
        {
            self.visual_refs(session_id, &mut model).await;
        }
        self.service
            .reading_allowed(session_id, &page, &model.url)?;
        if self.service.page_mount(session_id).as_ref() != Some(&mount) {
            return Err(PAGE_CLOSED.to_owned());
        }
        // Keep the receipt's target first, then priority order (including page-covering consent dialogs), and reserve half the
        // slots for main-document candidates. Select before bound's absolute 200-ref cap too.
        if let Some(selector) = &options.selector {
            if let Some(index) = selector.index {
                model.refs = model.refs.get(index).cloned().into_iter().collect();
            }
        } else {
            model.refs.sort_by_key(|item| {
                std::cmp::Reverse((Some(item.id.as_str()) == priority_ref, item.priority))
            });
        }
        model.refs.retain(|item| crate::model::is_ref(&item.id));
        if options.selector.is_some() {
            model.refs.truncate(max_refs);
        } else {
            reserve_main_refs(&mut model.refs, max_refs);
        }
        crate::text_paging::select(&mut model, options.text_cursor.as_ref());
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
