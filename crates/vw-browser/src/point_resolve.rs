//! Read-only point resolution reuses the native click's observed node, geometry and frame guards.
//! Called by host action dispatch; returns page labels as untrusted content and never arms input listeners.
use crate::driver::refused;
use crate::{ActError, Action, Actor, Browser};
use serde_json::{Value, json};

impl Browser {
    pub(crate) async fn resolve_observed_point(
        &self,
        session: &str,
        x: u32,
        y: u32,
        by: Actor,
    ) -> Result<Value, ActError> {
        let action = self.bind_point(
            session,
            Action::ClickAt {
                reference: String::new(),
                x,
                y,
            },
        )?;
        self.check_ref(session, &action)?;
        let seen = self.current_seen(session).ok_or_else(|| {
            refused("No current point observation. Read the page before resolving a point.")
        })?;
        let item = seen
            .model
            .refs
            .iter()
            .find(|item| Some(item.id.as_str()) == action.reference())
            .ok_or_else(|| {
                refused("The point has no current observed target. Read the page again.")
            })?;
        let frame=item.frame_id.as_ref().ok_or_else(||refused("This engine has no isolated frame identity for the point. Use an observed element ref instead."))?;
        let mut checks = Vec::new();
        let mut ids = Vec::new();
        let (mut local_x, mut local_y) = (f64::from(x), f64::from(y));
        for ancestor in &frame.ancestors {
            checks.push(json!({"document":ancestor.document,"node":ancestor.node,"bounds":ancestor.bounds,"viewport":ancestor.viewport,"x":local_x,"y":local_y}));
            ids.push(ancestor.id.clone());
            local_x = (local_x - f64::from(ancestor.content.x))
                * f64::from(ancestor.child_viewport.0)
                / f64::from(ancestor.content.width);
            local_y = (local_y - f64::from(ancestor.content.y))
                * f64::from(ancestor.child_viewport.1)
                / f64::from(ancestor.content.height);
        }
        checks.push(json!({"target":true,"document":frame.document,"node":item.node,"bounds":item.local_bounds,"viewport":item.local_viewport,"x":local_x,"y":local_y}));
        ids.push(frame.id.clone());
        let page = self
            .service
            .page_accessor(session)
            .ok_or_else(|| refused("The observed page closed. Read the page again."))?;
        self.service
            .frame_reading_allowed(session, &page, &frame.url)
            .map_err(refused)?;
        let request = json!({"page":page,"checks":checks,"ref":item.id,"resolveOnly":true,"click":vw_preview::ClickOptions::HOVER});
        let answers=self.service.evaluate_frames_on_mount(&seen.mount,None,crate::scripts::point_validation(&request),self.dispatch_permission_check(session,by)).await.map_err(|e|refused(e.to_string()))?.ok_or_else(||refused("This engine cannot verify the point’s frame. Use an observed element ref instead."))?;
        for (id, check) in ids.iter().zip(&checks) {
            let matching: Vec<_> = answers.iter().filter(|answer| &answer.id == id).collect();
            if matching.len() != 1 {
                return Err(refused(
                    "The point’s frame changed. Read the page again before resolving it.",
                ));
            }
            let raw: Value = serde_json::from_str(&matching[0].value).unwrap_or(Value::Null);
            let answer = raw
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(raw);
            if answer["ok"] != true || answer["document"] != check["document"] {
                return Err(refused(
                    "The node, frame or geometry under the observed point changed. Read the page again before resolving it.",
                ));
            }
        }
        self.reading_ref_allowed(session, &page, &frame.url, action.reference())
            .map_err(refused)?;
        let mut receipt = self.host(session);
        receipt["ok"] = json!(true);
        receipt[crate::driver::HEADLINE_KEY] = json!("point resolved");
        receipt["result"] =
            json!({"action":"resolve_point","checked":true,"ref":item.id,"x":x,"y":y});
        receipt["snapshot"] = json!(seen.snapshot);
        receipt["untrustedPageContent"] = json!({"target":{"ref":item.id,"role":item.role,"name":item.name,"bounds":item.bounds},"url":seen.model.url});
        Ok(receipt)
    }
}
