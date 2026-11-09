//! Native pointer sequences for the Chromium browser fixture, including button/count/modifiers and picker guards.
//! Called by the shared test engine; uses its JSON pipe and isolated `DevTools` transport.
use super::cdp::{CdpRequest, PICKERS, cdp_frames, cdp_worker};
use super::{NATIVE_CLICK_WITHOUT_WINDOW, PageEngine};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use vw_preview::EngineError;

impl PageEngine {
    /// The agent's native click; CDP mode opens the act window's picker interception around it, as the `WebView2`
    /// adapter does for every act, and reports a picker it intercepted.
    pub(super) async fn agent_click(
        &self,
        label: &str,
        point: vw_preview::NativeClickPoint,
        guard: &vw_preview::NativeClickGuard,
    ) -> Result<(), EngineError> {
        let options = guard
            .validation
            .as_deref()
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .map(|request| request["click"].clone());
        if options
            .as_ref()
            .is_some_and(|options| options["count"] == 0)
        {
            self.rpc(json!({"op":"hover", "x":point.x, "y":point.y}))
                .await?;
            return Ok(());
        }
        let pickers = PICKERS.load(Ordering::Relaxed);
        let window = cdp_frames() && !self.page_world.load(Ordering::Relaxed);
        if window && !NATIVE_CLICK_WITHOUT_WINDOW.load(Ordering::Relaxed) {
            self.intercept(true).await;
        }
        self.rpc(json!({"op":"click", "x":point.x, "y":point.y,"options":options}))
            .await?;
        if window {
            self.intercept(false).await;
            if PICKERS.load(Ordering::Relaxed) > pickers {
                self.hooks()
                    .act_refused(label, vw_preview::ActRefusal::FilePicker);
            }
        }
        Ok(())
    }

    /// The act window's picker interception (CDP mode only; the picker event comes with the switch-off's answer).
    async fn intercept(&self, on: bool) {
        let (done, confirmed) = tokio::sync::oneshot::channel();
        let sent = self
            .cdp
            .lock()
            .unwrap()
            .get_or_insert_with(|| cdp_worker(self.pipe.clone()))
            .send(CdpRequest::Intercept(on, done));
        if sent.is_ok() {
            let _ = confirmed.await;
        }
    }
}
