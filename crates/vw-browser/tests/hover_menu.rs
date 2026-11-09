//! Real CSS hover and mouse events through `Browser::act` and the engine pointer seam.
//! Uses the shared Chromium page fixture; the same HTML is exercised by the native browser driver.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_opens_a_css_only_menu_and_delivers_real_mouse_events_without_clicking() {
    let rig = Rig::new().await;
    let html = serde_json::to_string(include_str!("fixtures/hover.html")).unwrap();
    rig.engine
        .eval(&format!(
            "document.open(); document.write({html}); document.close()"
        ))
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "Products").unwrap();
    assert_eq!(
        rig.engine.eval("getComputedStyle(choices).display").await,
        "none"
    );
    let action = vw_browser::parse(&serde_json::json!({"action":"hover","ref":target.id})).unwrap();
    rig.browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("getComputedStyle(choices).display").await,
        "block",
        "CSS :hover menu must open"
    );
    let events = rig.engine.eval("hoverEvents").await;
    for name in ["mouseover", "mouseenter", "mousemove"] {
        assert!(
            events
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == name && e["trusted"] == true),
            "{name}: {events}"
        );
    }
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .all(|e| !["mousedown", "mouseup", "click"]
                .iter()
                .any(|name| e["name"] == *name)),
        "hover clicked: {events}"
    );
    let (after, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(after.refs.iter().any(|r| r.name == "Widgets"));
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_refuses_a_replaced_or_covered_target_before_native_motion() {
    for change in [
        "products.replaceWith(products.cloneNode(true))",
        "document.body.insertAdjacentHTML('beforeend','<div style=\"position:fixed;inset:0;background:white\"></div>')",
    ] {
        let rig = Rig::new().await;
        let html = serde_json::to_string(include_str!("fixtures/hover.html")).unwrap();
        rig.engine
            .eval(&format!(
                "document.open();document.write({html});document.close()"
            ))
            .await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let target = model.refs.iter().find(|r| r.name == "Products").unwrap();
        rig.engine.eval(change).await;
        let action =
            vw_browser::parse(&serde_json::json!({"action":"hover","ref":target.id})).unwrap();
        assert!(
            rig.browser
                .act(SESSION, action, Actor::Agent, &NoNavigation)
                .await
                .is_err()
        );
        assert_eq!(
            rig.engine
                .native_clicks
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            rig.engine.eval("getComputedStyle(choices).display").await,
            "none"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_over_a_submitter_does_not_send_a_form_or_require_send_permission() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form method=post><button>Submit</button></form>';window.sends=0;document.querySelector('form').onsubmit=e=>{e.preventDefault();sends++}").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "Submit").unwrap();
    let action = vw_browser::parse(&serde_json::json!({"action":"hover","ref":target.id})).unwrap();
    rig.browser
        .act_with_sending(SESSION, action, Actor::Agent, &NoNavigation, false)
        .await
        .unwrap();
    assert_eq!(rig.engine.eval("sends").await, 0);
}
