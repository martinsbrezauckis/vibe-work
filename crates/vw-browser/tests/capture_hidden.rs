//! Passive HTML saves preserve content hidden by native or adopted styles outside document.styleSheets.
//! Uses the real capture script and Chromium fixture, then reopens the saved document without the original sheet.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;
use serde_json::{Value, json};
use support::Rig;

#[tokio::test]
async fn reopening_saved_html_does_not_resurrect_hidden_advertisements() {
    for format in ["html", "single_html"] {
        let rig = Rig::new().await;
        rig.engine.eval("document.body.innerHTML = '<div class=advert>Advertisement</div><p>Article</p>'; const sheet = new CSSStyleSheet(); sheet.replaceSync('.advert { display: none !important }'); document.adoptedStyleSheets = [sheet];").await;
        let script = vw_browser::scripts::page_text(
            "capture-fixture",
            &json!({
                "op":"start", "format":format, "token":"capture", "maxChars":100_000, "chunk":100_000
            }),
        );
        let result = rig.engine.eval(&script).await;
        let result: Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
        let text = result["chunk"].as_str().unwrap();
        let html = if format == "single_html" {
            serde_json::from_str::<Value>(text).unwrap()["html"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            text.to_owned()
        };
        rig.engine.eval(&format!("document.adoptedStyleSheets = []; document.open(); document.write({}); document.close();", json!(html))).await;
        assert_eq!(
            rig.engine
                .eval("getComputedStyle(document.querySelector('.advert')).display")
                .await,
            "none",
            "{format}"
        );
        assert_eq!(
            rig.engine
                .eval("document.querySelector('p').innerText")
                .await,
            "Article"
        );
    }
}
