//! PDF plugin documents expose pixels but no DOM text; captures must explain that empty text body.
//! Uses the production page-text script in Chromium; ordinary pages and readable PDF text stay intact.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;

use serde_json::{Value, json};
use support::Rig;

#[tokio::test]
async fn pdf_viewer_without_dom_text_saves_a_reason_and_next_step() {
    let rig = Rig::new().await;
    for (body, expected) in [
        (
            "<embed type='application/pdf' src='about:blank'>",
            "This PDF viewer does not expose page text. Use a screenshot to read the page.",
        ),
        (
            "<object type='application/pdf'></object>",
            "This PDF viewer does not expose page text. Use a screenshot to read the page.",
        ),
        (
            "<embed type='application/pdf'><p>Readable PDF text</p>",
            "Readable PDF text",
        ),
        ("", ""),
    ] {
        rig.engine
            .eval(&format!("document.body.innerHTML = {}", json!(body)))
            .await;
        let script = vw_browser::scripts::page_text(
            "capture-fixture",
            &json!({
                "op":"start", "format":"text", "token":"pdf", "maxChars":100_000, "chunk":100_000
            }),
        );
        let answer = rig.engine.eval(&script).await;
        let answer: Value = serde_json::from_str(answer.as_str().unwrap()).unwrap();
        assert_eq!(answer["chunk"].as_str().unwrap().trim(), expected, "{body}");
    }
}
