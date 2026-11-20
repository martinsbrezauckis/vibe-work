//! Frame fixture setup waits for native load events, with pending-frame diagnostics on timeout.
//! Uses `PageEngine` evaluation; called before browser observation assertions in integration tests.
use super::PageEngine;
use serde_json::{Value, json};
use vw_preview::{EngineError, HistoryStep, PageEvent};

impl PageEngine {
    pub async fn eval_frames(&self, script: &str) {
        let source = format!(
            r#"new Promise((resolve, reject) => {{
          const loaded = new WeakSet();
          let frames = null;
          const clean = () => {{ clearTimeout(timer); document.removeEventListener('load', onLoad, true); }};
          const done = () => {{
            if (frames !== null && frames.every(frame => loaded.has(frame))) {{ clean(); resolve(frames.map(frame => frame.hasAttribute("srcdoc") ? "about:srcdoc" : frame.src || "about:blank")); }}
          }};
          const onLoad = event => {{ if (event.target instanceof HTMLIFrameElement) loaded.add(event.target); done(); }};
          document.addEventListener('load', onLoad, true);
          const timer = setTimeout(() => {{
            const pending = (frames || Array.from(document.querySelectorAll('iframe'))).filter(frame => !loaded.has(frame)).map(frame => frame.outerHTML);
            clean(); reject(new Error('frame fixture load timed out: ' + JSON.stringify(pending)));
          }}, 10000);
          Promise.resolve().then(() => (0, eval)({})).then(() => {{
            frames = Array.from(document.querySelectorAll('iframe')); done();
          }}, error => {{ clean(); reject(error); }});
        }})"#,
            serde_json::json!(script)
        );
        let addresses = self.eval(&source).await;
        let Value::Array(addresses) = addresses else {
            panic!("expected loaded frame addresses, got {addresses}");
        };
        for address in addresses {
            let Value::String(address) = address else {
                panic!("expected a frame address, got {address}");
            };
            self.wait_for_frame(&address, "body").await;
        }
    }

    /// Hands the child-frame documents the page asked for since the last call to the navigation hook, as the engines
    /// ask it before a frame document loads. Called before every all-frames evaluation.
    pub(crate) async fn report_frame_navigations(&self, label: &str) -> Result<(), EngineError> {
        let urls = self.rpc(json!({"op":"frameNavigations"})).await?;
        for url in urls.as_array().into_iter().flatten() {
            if let Some(url) = url.as_str().and_then(|url| url.parse().ok()) {
                self.hooks().allow_frame_navigation(label, &url);
            }
        }
        Ok(())
    }

    /// A reload loads the page again, reported to the hooks as the engines report a load; back and forward do
    /// nothing here.
    pub(crate) async fn fixture_history(
        &self,
        label: String,
        step: HistoryStep,
    ) -> Result<(), EngineError> {
        if !matches!(step, HistoryStep::Reload | HistoryStep::ReloadBypassCache) {
            return Ok(());
        }
        let url = self
            .rpc(json!({"op":"evaluate","script":"location.href"}))
            .await?;
        let url: url::Url = url
            .as_str()
            .and_then(|u| u.parse().ok())
            .ok_or_else(|| EngineError::Platform("no page address".into()))?;
        self.hooks()
            .page_event(&label, PageEvent::Started(url.clone()));
        self.rpc(json!({"op":"reload"})).await?;
        self.hooks().page_event(&label, PageEvent::Finished(url));
        Ok(())
    }
}
