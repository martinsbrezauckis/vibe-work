//! Frame readiness for browser test fixtures, through `PageEngine`'s JSON pipe.
//! Called before observing or reusing a frame target; never refreshes the browser's observed refs.

use super::PageEngine;
use serde_json::json;

impl PageEngine {
    /// Waits for the fixture document and its control, without refreshing the browser's observed refs.
    pub async fn wait_for_frame(&self, url: &str, selector: &str) {
        self.rpc(json!({"op":"waitForFrame", "url":url, "selector":selector}))
            .await
            .unwrap_or_else(|error| {
                panic!("waiting for loaded frame {url} containing {selector}: {error}")
            });
    }
}
