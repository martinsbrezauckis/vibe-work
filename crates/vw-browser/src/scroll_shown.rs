//! `scroll {ref}` past the element's own document: a control inside another site's frame is scrolled into its
//! frame's view by the act script, but the engine keeps that frame from moving the documents around it. The host
//! then scrolls each ancestor document (innermost first) so the element shows, and refuses a "done" for an element
//! still off screen.
//!
//! Depends on: the current observation's frame ancestry, `scripts::frame_scroll`, `frames::run_frame`. Called by:
//! `driver::perform` after a scroll act.

use serde::Deserialize;
use serde_json::json;

use crate::act::{Action, PageAnswer};
use crate::driver::{ActError, Browser, failed, refused};
use crate::frames::{FrameRef, RefBox};
use crate::refs::PAGE_CLOSED;
use crate::sessions::Actor;

#[derive(Deserialize)]
struct Moved {
    status: String,
    #[serde(rename = "box")]
    bounds: Option<RefBox>,
    viewport: Option<(u32, u32)>,
}

impl Browser {
    /// After `scroll {ref}` answered done: moves the ancestor documents of a frame element so it shows. `Err`
    /// when the element is still outside the page's viewport (the answer says the page did not move to it).
    pub(crate) async fn scroll_shown(
        &self,
        session_id: &str,
        action: &Action,
        answer: &PageAnswer,
        by: Actor,
    ) -> Result<(), ActError> {
        let Action::Scroll {
            reference: Some(reference),
            ..
        } = action
        else {
            return Ok(());
        };
        let (Some(mut bounds), Some(mut viewport)) = (answer.bounds, answer.viewport) else {
            return Ok(());
        };
        let ancestors = self
            .current_seen(session_id)
            .and_then(|seen| {
                seen.model
                    .refs
                    .iter()
                    .find(|r| r.id == *reference)
                    .and_then(|r| r.frame_id.as_ref())
                    .map(|frame| frame.ancestors.clone())
            })
            .unwrap_or_default();
        if !ancestors.is_empty() {
            let mount = self.ref_mount(session_id)?;
            let page = self
                .service
                .page_accessor(session_id)
                .ok_or_else(|| refused(PAGE_CLOSED))?;
            let allowed = self.dispatch_permission_check(session_id, by);
            for ancestor in ancestors.iter().rev() {
                let frame = FrameRef {
                    ancestors: Vec::new(),
                    id: ancestor.id.clone(),
                    document: ancestor.document.clone(),
                    origin: String::new(),
                    url: ancestor.url.clone(),
                };
                let script = crate::scripts::frame_scroll(
                    &json!({"page": page, "node": ancestor.node, "box": bounds}),
                );
                let raw = self
                    .run_frame(session_id, &mount, &frame, &script, allowed.clone())
                    .await
                    .map_err(failed)?;
                let moved: Moved = crate::model::parse_answer(&raw).map_err(failed)?;
                match (moved.status.as_str(), moved.bounds, moved.viewport) {
                    ("done", Some(b), Some(v)) => (bounds, viewport) = (b, v),
                    _ => return Err(not_moved(reference)),
                }
            }
        }
        // At least part of the element inside the page's viewport (an element without a box counts as its point).
        let (x, y) = (i64::from(bounds.x), i64::from(bounds.y));
        let shown = x < i64::from(viewport.0)
            && y < i64::from(viewport.1)
            && x + i64::from(bounds.width.max(1)) > 0
            && y + i64::from(bounds.height.max(1)) > 0;
        if shown {
            Ok(())
        } else {
            Err(not_moved(reference))
        }
    }
}

fn not_moved(reference: &str) -> ActError {
    failed(format!(
        "not done: the page did not move to {reference}; it is still off screen. Scroll the page (browser_act \
         scroll with direction down or up), then call browser_read again."
    ))
}
