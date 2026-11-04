//! Finds the live on-screen target after scripted input, using the existing isolated frame observation.
//! Called by the action driver; reports coordinates to the host only, never adds page nodes or events.

use crate::{ActError, Action, Actor, Browser, PageAnswer};
use serde_json::Value;
#[cfg(feature = "click-marker")]
use vw_preview::NativeClickPoint;

impl Browser {
    pub(crate) async fn finish_input(
        &self,
        session: &str,
        action: &Action,
        request: &Value,
        by: Actor,
        answer: PageAnswer,
    ) -> Result<PageAnswer, ActError> {
        #[cfg(feature = "click-marker")]
        if by == Actor::Agent
            && answer.status == "done"
            && request["nativeClick"] != true
            && matches!(action, Action::Click { .. } | Action::Type { .. })
        {
            self.mark_input(session, action).await;
        }
        if matches!(action, Action::Type { submit: true, .. }) {
            self.submit_checked_text(session, action, request, by).await
        } else {
            Ok(answer)
        }
    }

    #[cfg(feature = "click-marker")]
    pub(crate) async fn mark_input(&self, session: &str, action: &Action) {
        let Some(seen) = self.current_seen(session) else {
            return;
        };
        let Some(before) = action
            .reference()
            .and_then(|id| seen.items.iter().find(|r| r.id == id))
        else {
            return;
        };
        let Ok(model) = self.observe_unrecorded(session, Some(before.node)).await else {
            return;
        };
        let Some(after) = model.refs.iter().find(|r| {
            r.node == before.node
                && r.id == before.id
                && r.frame_id
                    .as_ref()
                    .zip(before.frame_id.as_ref())
                    .is_some_and(|(a, b)| a.id == b.id && a.document == b.document)
        }) else {
            return;
        };
        let (Some(bounds), Some((width, height))) = (after.bounds, model.viewport) else {
            return;
        };
        let left = f64::from(bounds.x).max(0.0);
        let top = f64::from(bounds.y).max(0.0);
        let right = (f64::from(bounds.x) + f64::from(bounds.width)).min(f64::from(width));
        let bottom = (f64::from(bounds.y) + f64::from(bounds.height)).min(f64::from(height));
        if right <= left || bottom <= top {
            return;
        }
        self.service
            .show_input_marker(
                &seen.mount,
                NativeClickPoint {
                    x: left.midpoint(right),
                    y: top.midpoint(bottom),
                    viewport_width: f64::from(width),
                    viewport_height: f64::from(height),
                },
            )
            .await;
    }
}
