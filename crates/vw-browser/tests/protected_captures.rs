//! Visual captures below Full access never show a protected field's value: a `browser_read` screenshot, a
//! `capture.screenshot`, a GIF frame and a full-page PNG are refused while the page shows a password, PIN or card
//! value (the shared privateField rule); a Full access chat (the core's delegation) captures as before.
//! Uses the real browser driver and Chromium pixels; fake values only.
//!
//! PREDICT (before the admission check): every capture below succeeds and its pixels show the typed card number
//! 4111111111119917 and the PIN; nothing on the visual path evaluated the privateField rule.
//! PREDICT (closed shadow roots, before the page-world keeper): a card number inside a closed root is captured below
//! Full access, because the admission's walk sees `shadowRoot === null` on the host and reports nothing shown.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use std::sync::Arc;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions, protected_shown};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

struct Full;
impl vw_browser::delegation::DelegationAuthorizer for Full {
    fn protected_values_delegated(&self, _: &str) -> bool {
        true
    }
}

async fn page(rig: &Rig, html: &str) {
    rig.engine
        .eval(&format!("document.body.innerHTML = {}", json!(html)))
        .await;
}

async fn screenshot(rig: &Rig) -> Result<serde_json::Value, String> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"capture.screenshot"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|error| error.text)
}

async fn read_shot(rig: &Rig) -> Result<(), String> {
    rig.browser
        .read(SESSION, ReadOptions::default(), true, Actor::Agent)
        .await
        .map(|_| ())
}

const SHOWN: [&str; 4] = [
    "<label>Card number <input value=4111111111119917></label>",
    "<label>PIN <input type=text value=4471></label>",
    "<div>PIN</div><div contenteditable=true>5678</div>",
    "<label>Card PIN list <select><option selected>PINSELTXT-4471</option></select></label>",
];

#[tokio::test]
async fn captures_below_full_access_are_refused_while_a_protected_value_shows() {
    let rig = Rig::new().await;
    for html in SHOWN {
        page(&rig, html).await;
        assert_eq!(
            screenshot(&rig).await.unwrap_err(),
            protected_shown(),
            "{html}"
        );
        assert_eq!(
            read_shot(&rig).await.unwrap_err(),
            protected_shown(),
            "{html}"
        );
        let full = vw_browser::capture::shot::full_page(&rig.browser, SESSION, None).await;
        assert_eq!(
            full.err().as_deref(),
            Some(protected_shown().as_str()),
            "{html}"
        );
    }
}

#[tokio::test]
async fn gif_frames_below_full_access_are_not_taken_while_a_protected_value_shows() {
    let rig = Rig::new().await;
    page(&rig, "<p>Ordinary page</p>").await;
    let act = |action: &'static str| {
        rig.browser.act(
            SESSION,
            vw_browser::parse(&json!({ "action": action })).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
    };
    let started = act("recording.start").await.unwrap();
    let before = started["recording"]["frames"].as_u64().unwrap();
    page(&rig, SHOWN[0]).await;
    act("media.read").await.unwrap();
    act("media.read").await.unwrap();
    let stopped = act("recording.stop").await.unwrap();
    assert_eq!(
        stopped["recording"]["frames"].as_u64().unwrap(),
        before,
        "no frame of the page showing the card number: {stopped}"
    );
}

#[tokio::test]
async fn masked_empty_and_ordinary_fields_do_not_stop_a_capture_and_full_access_delegates() {
    let rig = Rig::new().await;
    page(
        &rig,
        "<label>Password <input type=password value=PWVAL-1></label><label>Card number <input></label>\
         <label>Notes <input value=ordinary-note></label><input type=hidden value=HIDVAL-1>",
    )
    .await;
    screenshot(&rig).await.unwrap();
    read_shot(&rig).await.unwrap();
    page(&rig, SHOWN[0]).await;
    rig.browser.set_delegation_authorizer(Arc::new(Full));
    screenshot(&rig).await.unwrap();
    read_shot(&rig).await.unwrap();
    vw_browser::capture::shot::full_page(&rig.browser, SESSION, None)
        .await
        .unwrap();
}

async fn frame_page(rig: &Rig, inner: &str) {
    page(rig, &format!("<iframe srcdoc=\"{inner}\"></iframe>")).await;
    for _ in 0..40 {
        let ready = rig
            .engine
            .eval("document.querySelector('iframe').contentDocument?.readyState === 'complete' && document.querySelector('iframe').contentDocument.body.childElementCount > 0")
            .await;
        if ready == true {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the fixture frame did not load");
}

#[tokio::test]
async fn child_frames_are_checked_and_a_frame_that_cannot_be_checked_refuses() {
    let rig = Rig::new().await;
    frame_page(
        &rig,
        "<label>Card number <input value=4111111111119917></label>",
    )
    .await;
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
    frame_page(&rig, "<p>Ordinary frame</p>").await;
    screenshot(&rig).await.unwrap();
    rig.engine
        .unreadable_children
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
}

/// A closed shadow root made by the page's own script (page world, through `attachShadow`).
async fn closed_root_page(rig: &Rig, inner: &str) {
    page(rig, "<p>Checkout</p><div id=host></div>").await;
    let script = format!(
        "document.getElementById('host').attachShadow({{mode:'closed'}}).innerHTML = {}; true",
        json!(inner)
    );
    assert_eq!(rig.engine.eval(&script).await, true);
}

#[tokio::test]
async fn a_filled_field_in_a_closed_shadow_root_refuses_captures_below_full_access() {
    let rig = Rig::new().await;
    closed_root_page(
        &rig,
        "<label>Card number <input value=4111111111119917></label>",
    )
    .await;
    assert_eq!(
        rig.engine
            .eval("document.getElementById('host').shadowRoot === null")
            .await,
        true,
        "the fixture's root is closed"
    );
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
    assert_eq!(read_shot(&rig).await.unwrap_err(), protected_shown());
    let full = vw_browser::capture::shot::full_page(&rig.browser, SESSION, None).await;
    assert_eq!(full.err().as_deref(), Some(protected_shown().as_str()));
    // A protected select whose chosen option is its first one.
    closed_root_page(
        &rig,
        "<label>Card PIN list <select><option selected>PINSELTXT-4471</option></select></label>",
    )
    .await;
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
    // Text, a masked password and an empty card field in a closed root do not stop a capture.
    closed_root_page(
        &rig,
        "<p>Pay with card</p><label>Password <input type=password value=PWVAL-2></label>\
         <label>Card number <input></label>",
    )
    .await;
    screenshot(&rig).await.unwrap();
    // Full access delegates the page's data, closed roots included.
    closed_root_page(&rig, "<label>PIN <input value=4471></label>").await;
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
    rig.browser.set_delegation_authorizer(Arc::new(Full));
    screenshot(&rig).await.unwrap();
}

/// A host drawn as `display: contents` has no box of its own; its closed root's field still shows.
#[tokio::test]
async fn a_closed_root_of_a_display_contents_host_is_looked_at() {
    let rig = Rig::new().await;
    page(&rig, "<div id=host style='display:contents'></div>").await;
    let script = "document.getElementById('host').attachShadow({mode:'closed'}).innerHTML = \
        '<label>Card number <input value=4111111111119917></label>'; \
        document.getElementById('host').getClientRects().length === 0";
    assert_eq!(rig.engine.eval(script).await, true, "the host has no box");
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
}

#[tokio::test]
async fn an_open_root_inside_a_closed_root_is_looked_at_too() {
    let rig = Rig::new().await;
    page(&rig, "<div id=host></div>").await;
    let script = "const closed = document.getElementById('host').attachShadow({mode:'closed'}); \
        closed.innerHTML = '<div id=inner></div>'; \
        closed.getElementById('inner').attachShadow({mode:'open'}).innerHTML = \
        '<label>PIN <input value=4471></label>'; true";
    assert_eq!(rig.engine.eval(script).await, true);
    assert_eq!(screenshot(&rig).await.unwrap_err(), protected_shown());
}
