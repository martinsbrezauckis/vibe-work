//! A closed shadow root the HTML parser attaches (declarative shadow DOM: `<template shadowrootmode="closed">`, also
//! through `setHTMLUnsafe`, `parseHTMLUnsafe`, `document.write` and `srcdoc`) never passes the page-world keeper's
//! `attachShadow` hook. A card field inside one must still stop raw page scripts, raw data reads and visual captures
//! in every access mode; ordinary pages and open shadow roots keep their raw tools and captures.
//! Uses the real browser driver and Chromium (`VW_TEST_CDP_FRAMES=1`: the `WebView2` protocol route, which enters
//! closed roots; otherwise the fixture's page-world route, which relies on the keeper and the engine's source check).
//! Fake values only.
//!
//! A refused raw action marks its site (protected-field site memory), so every refused request below runs on a freshly
//! prepared page of its own origin: each one proves the page's own scan, not the memory of an earlier refusal.
//!
//! PREDICT (before this fix): every refusal below fails. The scan reports zero protected fields and nothing shown for
//! a parser-made closed root, so the raw script runs and the screenshot is taken while the card number is rendered.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, protected_shown};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

const CARD: &str =
    "<label>Card number <input autocomplete=\"cc-number\" value=\"4111111111119917\"></label>";

fn closed(inner: &str) -> String {
    format!(
        "<p>Checkout</p><div id=\"host\"><template shadowrootmode=\"closed\">{inner}</template></div>"
    )
}

static NEXT_SITE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A parser-built document on an origin no earlier page of the test used.
fn parsed_url(html: &str) -> String {
    let site = NEXT_SITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut url =
        url::Url::parse(&format!("https://s{site}.declarative.example.test/doc")).unwrap();
    url.query_pairs_mut().append_pair("html", html);
    url.into()
}

/// The same document as a path, for a same-origin child frame of the current page.
fn parsed_path(html: &str) -> String {
    let url = url::Url::parse(&parsed_url(html)).unwrap();
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

/// How a refused case builds its page; run again before every refused request.
enum Prep<'a> {
    /// The HTML parser builds the main document.
    Parsed(&'a str),
    /// The same document in a child frame of an ordinary page.
    Frame(&'a str),
    /// A page script builds it on an ordinary page.
    Script(&'a str),
}

async fn prepare(rig: &Rig, prep: &Prep<'_>) {
    match prep {
        Prep::Parsed(html) => goto(rig, &parsed_url(html)).await,
        Prep::Frame(html) => {
            goto(rig, &parsed_url("<p>Shop</p>")).await;
            rig.engine
                .eval(&format!(
                    "document.body.insertAdjacentHTML('beforeend', {})",
                    json!(format!("<iframe src=\"{}\"></iframe>", parsed_path(html)))
                ))
                .await;
            frame_loaded(rig).await;
        }
        Prep::Script(script) => {
            goto(rig, &parsed_url("<p>Shop</p>")).await;
            rig.engine.eval(script).await;
            if script.contains("srcdoc") {
                frame_loaded(rig).await;
            }
        }
    }
}

/// Opens `url` in the session as a navigation of the browser (the HTML parser builds its document).
async fn goto(rig: &Rig, url: &str) {
    rig.service
        .open(vw_preview::OpenRequest {
            session_id: SESSION.into(),
            url: url.parse().unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: true,
            lan: false,
        })
        .await
        .unwrap();
}

async fn act(rig: &Rig, request: Value) -> Result<Value, vw_browser::ActError> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

/// Raw script, raw data read, `capture.screenshot` and the read screenshot are all refused.
async fn all_refused(rig: &Rig, case: &str, prep: &Prep<'_>) {
    for request in [
        json!({"action":"raw.execute","expression":"window.rawRan = true"}),
        json!({"action":"storage.read","area":"local","keys":["fixture"]}),
        json!({"action":"cookies.read"}),
        json!({"action":"console.read"}),
    ] {
        prepare(rig, prep).await;
        let error = act(rig, request.clone()).await.unwrap_err();
        assert!(
            error.text.contains("Protected field values were withheld"),
            "{case}: {request} -> {}",
            error.text
        );
        assert!(
            error.protected_values_withheld.unwrap_or_default() > 0,
            "{case}"
        );
    }
    prepare(rig, prep).await;
    act(
        rig,
        json!({"action":"raw.execute","expression":"window.rawRan = true"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        rig.engine.eval("window.rawRan === undefined").await,
        json!(true),
        "{case}: the raw script ran"
    );
    let shot = act(rig, json!({"action":"capture.screenshot"})).await;
    assert_eq!(shot.unwrap_err().text, protected_shown(), "{case}");
    let read = rig
        .browser
        .read(SESSION, ReadOptions::default(), true, Actor::Agent)
        .await;
    assert_eq!(
        read.err().as_deref(),
        Some(protected_shown().as_str()),
        "{case}"
    );
}

async fn all_allowed(rig: &Rig, case: &str) {
    act(rig, json!({"action":"raw.execute","expression":"1 + 1"}))
        .await
        .unwrap_or_else(|error| panic!("{case}: raw script refused: {}", error.text));
    act(rig, json!({"action":"capture.screenshot"}))
        .await
        .unwrap_or_else(|error| panic!("{case}: capture refused: {}", error.text));
}

async fn frame_loaded(rig: &Rig) {
    for _ in 0..80 {
        let ready = rig
            .engine
            .eval("(() => { const d = document.querySelector('iframe')?.contentDocument; return d?.readyState === 'complete' && !!d.getElementById('host'); })()")
            .await;
        if ready == true {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the fixture frame did not load");
}

#[tokio::test]
async fn a_parsed_closed_root_card_field_refuses_raw_reads_and_captures() {
    let rig = Rig::new().await;
    goto(&rig, &parsed_url(&closed(CARD))).await;
    assert_eq!(
        rig.engine
            .eval("document.getElementById('host').shadowRoot === null && document.querySelectorAll('input, template').length === 0")
            .await,
        json!(true),
        "the parser attached a closed root"
    );
    all_refused(&rig, "parsed main document", &Prep::Parsed(&closed(CARD))).await;
    all_refused(&rig, "parsed child frame", &Prep::Frame(&closed(CARD))).await;
}

#[tokio::test]
async fn script_made_declarative_roots_refuse_raw_reads_and_captures() {
    let rig = Rig::new().await;
    let cases = [
        (
            "setHTMLUnsafe",
            format!("document.body.setHTMLUnsafe({})", json!(closed(CARD))),
        ),
        (
            "parseHTMLUnsafe",
            format!(
                "document.body.append(Document.parseHTMLUnsafe({}).getElementById('host'))",
                json!(closed(CARD))
            ),
        ),
        (
            "split document.write",
            format!(
                "document.open(); document.write('<p>Checkout</p><div id=host><template shadowro'); document.write({}); document.close()",
                json!(format!("otmode=closed>{CARD}</template></div>"))
            ),
        ),
        (
            "srcdoc frame",
            format!(
                "document.body.innerHTML = {}",
                json!(format!(
                    "<iframe srcdoc=\"{}\"></iframe>",
                    closed(CARD).replace('"', "&quot;")
                ))
            ),
        ),
    ];
    for (case, script) in cases {
        all_refused(&rig, case, &Prep::Script(&script)).await;
    }
}

#[tokio::test]
async fn ordinary_pages_and_open_declarative_roots_keep_raw_tools_and_captures() {
    let rig = Rig::new().await;
    goto(
        &rig,
        &parsed_url("<p>Shop</p><label>Name <input value=Ada></label>"),
    )
    .await;
    all_allowed(&rig, "ordinary page").await;
    let open = "<div id=host><template shadowrootmode=\"open\"><label>Name <input value=Ada></label></template></div>";
    goto(&rig, &parsed_url(open)).await;
    assert_eq!(
        rig.engine
            .eval("document.getElementById('host').shadowRoot !== null")
            .await,
        json!(true)
    );
    all_allowed(&rig, "open declarative root").await;
    goto(&rig, &parsed_url("<p>Shop</p>")).await;
    rig.engine
        .eval(&format!("document.body.setHTMLUnsafe({})", json!(open)))
        .await;
    all_allowed(&rig, "open root through setHTMLUnsafe").await;
    // A card field inside an open declarative root is an ordinary protected field.
    all_refused(
        &rig,
        "card field in an open declarative root",
        &Prep::Parsed(&open.replace("<label>Name <input value=Ada></label>", CARD)),
    )
    .await;
}

#[tokio::test]
async fn a_closed_root_without_fields_is_inspected_where_the_engine_enters_closed_roots() {
    let rig = Rig::new().await;
    let plain = closed("<p>Plain text only</p>");
    goto(&rig, &parsed_url(&plain)).await;
    let cdp = std::env::var_os("VW_TEST_CDP_FRAMES").is_some_and(|v| v == "1");
    if cdp {
        // The protocol route reads the closed root itself: nothing protected, nothing withheld.
        all_allowed(&rig, "closed root without fields (protocol route)").await;
    } else {
        // Without a world that enters closed roots, a parser-made closed root fails closed.
        all_refused(
            &rig,
            "closed root without fields (no closed-root access)",
            &Prep::Parsed(&plain),
        )
        .await;
    }
}
