//! Frame fixture setup waits for native load events, with pending-frame diagnostics on timeout.
//! Uses `PageEngine` evaluation; called before browser observation assertions in integration tests.
use super::PageEngine;
use serde_json::Value;

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
}
