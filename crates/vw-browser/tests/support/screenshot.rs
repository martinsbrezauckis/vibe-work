//! Device-scale fixtures for screenshot receipts; uses the real Chromium capture behind `PageEngine`.
//! Also what happens to the page right after a capture: a host strip resizing the view, a script, a navigation.
use super::{PageEngine, PageEvent, URL};
use serde_json::json;
use std::sync::atomic::Ordering;
use vw_preview::MountId;

/// A change applied to the page right after one capture, in queue order (one per capture).
pub enum AfterShot {
    /// The view's CSS size changes, as when the app shows or hides a strip above the preview.
    Resize(u32, u32),
    /// A script runs in the page.
    Eval(&'static str),
}

impl PageEngine {
    pub async fn scaled_viewport(&self, width: u32, height: u32, scale: f64) {
        self.rpc(json!({"op":"viewport", "width":width, "height":height, "scale":scale}))
            .await
            .unwrap();
    }

    /// Queues changes for the next captures, one each.
    pub fn after_shots(&self, changes: impl IntoIterator<Item = AfterShot>) {
        self.after_shot.lock().unwrap().extend(changes);
    }

    /// Changes still queued (the captures that have not happened).
    pub fn after_shots_left(&self) -> usize {
        self.after_shot.lock().unwrap().len()
    }

    pub(super) async fn during_shot(&self, id: &MountId) {
        if self.navigate_during_shot.swap(false, Ordering::Relaxed) {
            self.hooks
                .get()
                .unwrap()
                .page_event(&id.label, PageEvent::Started(URL.parse().unwrap()));
        }
        let next = self.after_shot.lock().unwrap().pop_front();
        match next {
            Some(AfterShot::Resize(width, height)) => {
                self.rpc(json!({"op":"viewport", "width":width, "height":height}))
                    .await
                    .unwrap();
            }
            Some(AfterShot::Eval(script)) => {
                self.eval(script).await;
            }
            None => {}
        }
    }
}
