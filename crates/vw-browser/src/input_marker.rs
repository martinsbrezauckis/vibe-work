//! Adapts completed scripted input to the common host action timeline, using isolated frame observations.
//! Called by the action driver; no marker nodes or events enter the page and no typed values enter the timeline.

use crate::{ActError, Action, Actor, Browser, PageAnswer};
use serde_json::Value;
#[cfg(feature = "click-marker")]
use vw_preview::action_tracking::{ActionBox, ActionGeometry, ActionKind};

impl Browser {
    #[cfg(feature = "click-marker")]
    pub(crate) async fn mark_native_input(
        &self,
        seen: &crate::sessions::Seen,
        item: &crate::model::RefItem,
        point: (u32, u32),
    ) {
        use vw_preview::action_tracking::ActionPoint;
        let (Some(frame), Some(loads), Some(viewport)) =
            (&item.frame_id, seen.loads, seen.model.viewport)
        else {
            return;
        };
        self.service
            .record_input(
                &seen.mount,
                &format!("{}:{}", frame.id, frame.document),
                loads,
                ActionKind::Click,
                ActionGeometry {
                    point: Some(ActionPoint {
                        x: f64::from(point.0),
                        y: f64::from(point.1),
                    }),
                    target_box: item.bounds.map(|bounds| ActionBox {
                        x: f64::from(bounds.x),
                        y: f64::from(bounds.y),
                        width: f64::from(bounds.width),
                        height: f64::from(bounds.height),
                    }),
                    viewport: Some((f64::from(viewport.0), f64::from(viewport.1))),
                },
            )
            .await;
    }

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
        let Some(load) = self.service.load_state(session) else {
            return;
        };
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
        let Some(frame) = &after.frame_id else {
            return;
        };
        self.service
            .record_input(
                &seen.mount,
                &format!("{}:{}", frame.id, frame.document),
                load.loads,
                if matches!(action, Action::Type { .. }) {
                    ActionKind::Type
                } else {
                    ActionKind::Click
                },
                ActionGeometry {
                    point: None,
                    target_box: Some(ActionBox {
                        x: f64::from(bounds.x),
                        y: f64::from(bounds.y),
                        width: f64::from(bounds.width),
                        height: f64::from(bounds.height),
                    }),
                    viewport: Some((f64::from(width), f64::from(height))),
                },
            )
            .await;
    }
}
