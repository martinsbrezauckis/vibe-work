//! Device-scale fixtures for screenshot receipts; uses the real Chromium capture behind `PageEngine`.
use super::PageEngine;
use serde_json::json;

impl PageEngine {
    pub async fn scaled_viewport(&self, width: u32, height: u32, scale: f64) {
        self.rpc(json!({"op":"viewport", "width":width, "height":height, "scale":scale}))
            .await
            .unwrap();
    }
}
