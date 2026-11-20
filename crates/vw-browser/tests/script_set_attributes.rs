//! Attributes a page's own script sets with plain `setAttribute` (an `xmlns` on an icon `<svg>`, `xml:lang`,
//! `xmlns:xlink`) carry no namespace, a combination the DOM refuses to recreate through `setAttributeNS`. The
//! reader's redacted selector copy must still copy them, so a large page (its walk truncated), a selector read and an
//! extraction keep reading the main document. Uses the Chromium test engine in the page world and an isolated world;
//! the Windows frame route runs it with `VW_TEST_CDP_FRAMES=1`.
//!
//! PREDICT (before the fix): every read here fails with "frame document changed, detached, or could not be
//! evaluated" on the `DevTools` route (on the plain route the evaluation error), because the selector copy calls
//! `setAttributeNS(null, "xmlns", ...)`, which throws `NamespaceError` in the main frame.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, Selector, SelectorKind};

/// Icons whose namespace declarations a script set, as icon libraries do, plus a script-set `xml:lang`.
const SCRIPT_ATTRIBUTES: &str = "(() => {
  for (let i = 0; i < 3; i++) {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
    svg.setAttribute('xmlns:xlink', 'http://www.w3.org/1999/xlink');
    svg.setAttribute('width', '12');
    svg.setAttribute('height', '12');
    document.body.prepend(svg);
  }
  const quote = document.createElement('p');
  quote.setAttribute('xml:lang', 'lv');
  quote.textContent = 'Script quote';
  document.body.prepend(quote);
})()";

async fn page(isolated: bool, body: &str) -> Rig {
    let rig = Rig::new_in_world(isolated).await;
    rig.engine
        .eval(&format!(
            "document.body.innerHTML = {}; scrollTo(0,0)",
            serde_json::to_string(body).unwrap()
        ))
        .await;
    rig.engine.eval(SCRIPT_ATTRIBUTES).await;
    rig
}

fn text(answer: &serde_json::Value) -> String {
    answer["untrustedPageContent"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_page_with_script_set_namespace_attributes_is_read() {
    for isolated in [false, true] {
        // Over the walk's node bound, so the reader reads through its redacted copy.
        let body = format!(
            "<h1>Article heading</h1><p>Article lead paragraph</p>{}",
            "<span>x</span>".repeat(12_500)
        );
        let rig = page(isolated, &body).await;
        let (answer, _) = rig
            .browser
            .read(SESSION, ReadOptions::default(), false, Actor::Agent)
            .await
            .unwrap_or_else(|error| panic!("isolated={isolated}: {error}"));
        assert!(
            text(&answer).contains("Article lead paragraph"),
            "isolated={isolated}: {answer}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selector_read_copies_script_set_namespace_attributes() {
    for isolated in [false, true] {
        let rig = page(isolated, "<button>Script page button</button>").await;
        let options = ReadOptions {
            selector: Some(Selector {
                kind: SelectorKind::Css,
                value: "svg[xmlns], p".into(),
                index: None,
            }),
            ..ReadOptions::default()
        };
        let (answer, _) = rig
            .browser
            .read(SESSION, options, false, Actor::Agent)
            .await
            .unwrap_or_else(|error| panic!("isolated={isolated}: {error}"));
        // Three icons matched by their copied `xmlns`, and the paragraph.
        assert_eq!(
            answer["matches"]["count"], 4,
            "isolated={isolated}: {answer}"
        );
    }
}
