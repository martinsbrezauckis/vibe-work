//! Coordinate input bound to the observed node and ancestor frames. Called by the driver.
//! Uses existing native input and receipts; coordinates never skip ownership or consent.
use crate::{
    Action, Browser, PageAnswer,
    driver::{ActError, failed, refused},
    sessions::Actor,
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

impl Browser {
    pub(crate) async fn native_ref_click(
        &self,
        session: &str,
        action: &Action,
        request: &serde_json::Value,
        point: vw_preview::NativeClickPoint,
        by: Actor,
        sent: Arc<AtomicBool>,
    ) -> Result<(), ActError> {
        let seen = self
            .current_seen(session)
            .ok_or_else(|| refused("page closed"))?;
        let item = action
            .reference()
            .and_then(|id| seen.items.iter().find(|r| r.id == id));
        if let Some(item) = item.filter(|r| r.frame_id.is_some()) {
            // Ordinary ref actions may scroll during preflight; coordinates retain their original geometry.
            let model = self.observe_dom(session).await.map_err(refused)?;
            let current = model
                .refs
                .iter()
                .find(|r| r.id == item.id)
                .filter(|current| same_node(item, current))
                .ok_or_else(|| refused("node or frame changed during native click preparation"))?;
            let (x, y) = top_point(current, point)?;
            self.click_point(session, &item.id, (x, y), by, sent, request.get("consent"))
                .await?;
            return Ok(());
        }
        Err(refused("native click requires an isolated frame identity"))
    }

    /// A direct coordinate click. The main document's load count does not change when only a child frame does, so
    /// the frames are observed again first, as for a native ref click: the target must still be the same node in
    /// the same frame, and a frame that cannot report the click's events now refuses it before any input. The click
    /// keeps the coordinates and geometry of the read they came from.
    pub(crate) async fn coordinate_click(
        &self,
        session: &str,
        reference: &str,
        point: (u32, u32),
        by: Actor,
        sent: Arc<AtomicBool>,
        consent: Option<&serde_json::Value>,
    ) -> Result<PageAnswer, ActError> {
        let seen = self
            .current_seen(session)
            .ok_or_else(|| refused("the observed page closed"))?;
        let item = seen
            .model
            .refs
            .iter()
            .find(|r| r.id == reference)
            .ok_or_else(|| refused("point ref is stale"))?;
        // Only the clicked node's refs are observed (a dense page's ref cap never leaves it out); every frame is.
        let model = self
            .observe_unrecorded(session, (item.node != 0).then_some(item.node))
            .await
            .map_err(refused)?;
        if model
            .unread_frames
            .iter()
            .any(|reason| reason == crate::frames::UNREADABLE)
        {
            return Err(refused(vw_preview::UNREPORTING_FRAME_REFUSAL));
        }
        // A shape the read found inside a canvas or image is no node of its own: its canvas or image must remain.
        if item.frame_id.is_some()
            && !model.refs.iter().any(|current| {
                (current.id == item.id || item.coordinate_kind.is_some())
                    && same_node(item, current)
            })
        {
            return Err(refused(
                "node or frame changed during native click preparation",
            ));
        }
        self.click_point(session, reference, point, by, sent, consent)
            .await
    }

    pub(crate) fn bind_point(&self, session: &str, action: Action) -> Result<Action, ActError> {
        // A shape the read found inside a canvas or image is no element of its own: a scroll to it scrolls its
        // canvas or image into view.
        let action = if let Action::Scroll {
            reference: Some(reference),
            up,
            amount,
        } = &action
            && let Some(seen) = self.current_seen(session)
            && let Some(shape) = seen.model.refs.iter().find(|r| {
                r.id == *reference
                    && r.coordinate_kind
                        .as_deref()
                        .is_some_and(|kind| kind != "region")
            })
            && let Some(region) = seen.model.refs.iter().find(|r| {
                r.coordinate_kind.as_deref() == Some("region")
                    && shape.node != 0
                    && r.node == shape.node
                    && r.frame_id == shape.frame_id
            }) {
            Action::Scroll {
                reference: Some(region.id.clone()),
                up: *up,
                amount: *amount,
            }
        } else {
            action
        };
        let action = if let Action::Click { reference } = &action {
            let seen = self.current_seen(session);
            let region = seen.as_ref().and_then(|s| {
                s.model
                    .refs
                    .iter()
                    .find(|r| r.id == *reference && r.coordinate_kind.is_some())
            });
            if let Some(bounds) = region.and_then(|r| r.bounds) {
                let x = u32::try_from(bounds.x + bounds.width / 2)
                    .map_err(|_| refused("coordinate ref is outside the viewport"))?;
                let y = u32::try_from(bounds.y + bounds.height / 2)
                    .map_err(|_| refused("coordinate ref is outside the viewport"))?;
                Action::ClickAt {
                    reference: reference.clone(),
                    x,
                    y,
                }
            } else {
                action
            }
        } else {
            action
        };
        let Action::ClickAt { reference, x, y } = &action else {
            return Ok(action);
        };
        if !reference.is_empty() {
            return Ok(action);
        }
        let seen = self
            .current_seen(session)
            .ok_or_else(|| refused("no current screenshot observation"))?;
        let item = seen
            .model
            .refs
            .iter()
            .filter(|r| {
                r.bounds.is_some_and(|b| {
                    i64::from(*x) >= i64::from(b.x)
                        && i64::from(*y) >= i64::from(b.y)
                        && i64::from(*x) < i64::from(b.x) + i64::from(b.width)
                        && i64::from(*y) < i64::from(b.y) + i64::from(b.height)
                })
            })
            .min_by_key(|r| r.bounds.map(|b| i64::from(b.width) * i64::from(b.height)))
            .ok_or_else(|| refused("no observed node at this point"))?;
        Ok(Action::ClickAt {
            reference: item.id.clone(),
            x: *x,
            y: *y,
        })
    }

    pub(crate) async fn click_point(
        &self,
        session: &str,
        reference: &str,
        point: (u32, u32),
        by: Actor,
        sent: Arc<AtomicBool>,
        consent: Option<&serde_json::Value>,
    ) -> Result<PageAnswer, ActError> {
        let seen = self
            .current_seen(session)
            .ok_or_else(|| refused("the observed page closed"))?;
        // A frame the read could not observe cannot report where the click's events went. Engines differ in how
        // such a frame reaches the click's own frame registry (Windows answers it as unreadable; WebKit on macOS can
        // answer the click's script from a frame whose observation failed), so the read's verdict decides, before
        // any input, the same way on every engine.
        if seen
            .model
            .unread_frames
            .iter()
            .any(|reason| reason == crate::frames::UNREADABLE)
        {
            return Err(refused(vw_preview::UNREPORTING_FRAME_REFUSAL));
        }
        let item = seen
            .model
            .refs
            .iter()
            .find(|r| r.id == reference)
            .ok_or_else(|| refused("point ref is stale"))?;
        let frame = item
            .frame_id
            .as_ref()
            .ok_or_else(|| refused("frame identity is unavailable for this point"))?;
        let viewport = seen
            .model
            .viewport
            .ok_or_else(|| refused("viewport identity is unavailable"))?;
        let (mut x, mut y) = (f64::from(point.0), f64::from(point.1));
        let mut checks = Vec::new();
        let mut ids = Vec::new();
        for ancestor in &frame.ancestors {
            checks.push(
                json!({"document":ancestor.document,"node":ancestor.node,"bounds":ancestor.bounds,
                "viewport":ancestor.viewport,"x":x,"y":y}),
            );
            ids.push(ancestor.id.clone());
            x = (x - f64::from(ancestor.content.x)) * f64::from(ancestor.child_viewport.0)
                / f64::from(ancestor.content.width);
            y = (y - f64::from(ancestor.content.y)) * f64::from(ancestor.child_viewport.1)
                / f64::from(ancestor.content.height);
        }
        checks.push(json!({"target":true,"document":frame.document,"node":item.node,"bounds":item.local_bounds,
            "viewport":item.local_viewport,"x":x,"y":y}));
        ids.push(frame.id.clone());
        let page = self
            .service
            .page_accessor(session)
            .ok_or_else(|| refused("the page closed"))?;
        self.service
            .frame_reading_allowed(session, &page, &frame.url)
            .map_err(refused)?;
        let receipt = crate::point_receipt::next_token();
        let documents: Vec<_> = checks
            .iter()
            .map(|check| check["document"].clone())
            .collect();
        let request = json!({"receipt":receipt,"page":page,"checks":checks,"singleFrame":true,"ref":reference,"consent":consent,"nativeUser":by == Actor::User});
        let script = crate::scripts::point_validation(&request);
        let dispatch = crate::scripts::point_dispatch(&page, &receipt);
        let validation = json!({"framePointValidation":true,"script":script,"dispatch":dispatch,"ids":ids,"documents":documents,"receipt":receipt}).to_string();
        let input = self
            .service
            .native_click_on_mount(
                &seen.mount,
                vw_preview::NativeClickPoint {
                    x: f64::from(point.0),
                    y: f64::from(point.1),
                    viewport_width: f64::from(viewport.0),
                    viewport_height: f64::from(viewport.1),
                },
                validation,
                sent.clone(),
                self.dispatch_permission_check(session, by),
            )
            .await
            .map_err(|e| native_error(&e, &sent))?;
        crate::point_receipt::confirm(&input, &page, &receipt, &frame.id, &frame.document).await?;
        #[cfg(feature = "click-marker")]
        {
            // Release the input lease before the overlay takes the same mount lock. Drawing never changes
            // the confirmed outcome; the feature-off path retains the original lease lifetime.
            drop(input);
            if by == Actor::Agent {
                self.mark_native_input(&seen, item, point).await;
            }
        }
        serde_json::from_value(json!({"status":"done","url":frame.url}))
            .map_err(|e| refused(e.to_string()))
    }
}

fn same_node(before: &crate::model::RefItem, after: &crate::model::RefItem) -> bool {
    let (Some(old), Some(new)) = (&before.frame_id, &after.frame_id) else {
        return false;
    };
    before.node == after.node
        && old.id == new.id
        && old.document == new.document
        && old.url == new.url
        && old.origin == new.origin
        && old.ancestors.len() == new.ancestors.len()
        && old
            .ancestors
            .iter()
            .zip(&new.ancestors)
            .all(|(a, b)| a.id == b.id && a.document == b.document && a.node == b.node)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn top_point(
    item: &crate::model::RefItem,
    point: vw_preview::NativeClickPoint,
) -> Result<(u32, u32), ActError> {
    let frame = item
        .frame_id
        .as_ref()
        .ok_or_else(|| refused("frame identity unavailable"))?;
    let (mut x, mut y) = (point.x, point.y);
    for ancestor in frame.ancestors.iter().rev() {
        x = f64::from(ancestor.content.x)
            + x * f64::from(ancestor.content.width) / f64::from(ancestor.child_viewport.0);
        y = f64::from(ancestor.content.y)
            + y * f64::from(ancestor.content.height) / f64::from(ancestor.child_viewport.1);
    }
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=100_000.0).contains(&x)
        || !(0.0..=100_000.0).contains(&y)
    {
        return Err(refused("point outside viewport"));
    }
    Ok((x.round() as u32, y.round() as u32))
}

fn native_error(error: &vw_preview::PreviewError, sent: &AtomicBool) -> ActError {
    if sent.load(std::sync::atomic::Ordering::Acquire) {
        failed(format!(
            "Native mouse input was sent but completion failed: {error}"
        ))
    } else if matches!(
        error,
        vw_preview::PreviewError::Engine(vw_preview::EngineError::FocusRequired)
    ) {
        refused(crate::native_focus::FOCUS_REQUEST)
    } else {
        refused(error.to_string())
    }
}
