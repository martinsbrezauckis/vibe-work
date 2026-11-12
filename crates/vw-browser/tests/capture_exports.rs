//! Shared passive capture scripts retain document identity, redact secret controls and disclose truncation.
//! Executes the same scripts used by the built-in driver and named extension compiler in real Chromium.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;
use serde_json::{Value, json};
use support::Rig;

#[tokio::test]
async fn capture_exports_redact_secrets_and_bind_chunks_to_the_document() {
    let rig = Rig::new().await;
    rig.engine.eval("window.captureFixture={};document.body.innerHTML='<h1>Article</h1><input type=password value=hidden-secret><textarea autocomplete=one-time-code>hidden-code</textarea><script>secretScript()</script><p onclick=bad()>Visible text</p>';").await;
    for format in ["html", "single_html", "markdown", "text"] {
        let script = vw_browser::scripts::page_text(
            "captureFixture",
            &json!({"op":"start","format":format,"token":"capture","maxChars":100_000,"chunk":100_000}),
        );
        let value = rig.engine.eval(&script).await;
        let data: Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
        let text = data["chunk"].as_str().unwrap();
        assert!(text.contains("Visible text"), "{format}");
        for secret in ["hidden-secret", "hidden-code", "secretScript", "onclick"] {
            assert!(!text.contains(secret), "{format}: {secret}");
        }
    }
    let script = vw_browser::scripts::page_text(
        "captureFixture",
        &json!({"op":"start","format":"text","token":"capture","maxChars":5,"chunk":5}),
    );
    let value = rig.engine.eval(&script).await;
    let data: Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(data["cut"], true);
    rig.engine.eval("delete window.captureFixture").await;
    let script =
        vw_browser::scripts::page_text("captureFixture", &json!({"op":"end","token":"capture"}));
    let value = rig.engine.eval(&script).await;
    let data: Value = serde_json::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(data["error"], "changed");
}
