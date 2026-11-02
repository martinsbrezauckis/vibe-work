//! Canvas/image regions become bounded coordinate refs without model calls.
//! Uses native screenshot bytes from the shared real-page engine; plain DOM pages skip the region pass.
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
async fn canvas_square_and_button_are_coordinate_refs() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<canvas width=280 height=160 style=\"position:fixed;left:0;top:0\"></canvas>'; scrollTo(0,0); const c=document.querySelector('canvas').getContext('2d'); c.fillStyle='white'; c.fillRect(0,0,280,160); c.strokeStyle='black'; c.lineWidth=2; c.strokeRect(20,20,20,20); c.strokeRect(80,20,100,40)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let value = serde_json::to_value(&model).unwrap();
    assert!(
        value["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["coordinateKind"] == "checkbox"),
        "missing square: {value}"
    );
    assert!(
        value["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["coordinateKind"] == "button"),
        "missing button: {value}"
    );
    let square = model
        .refs
        .iter()
        .find(|r| r.coordinate_kind.as_deref() == Some("checkbox"))
        .unwrap();
    rig.engine
        .eval("window.hits=0; document.querySelector('canvas').onclick=()=>window.hits++")
        .await;
    rig.browser
        .act(
            SESSION,
            vw_browser::Action::Click {
                reference: square.id.clone(),
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(rig.engine.eval("window.hits").await, 1);
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    // Region refs are shapes only: the agent's model reads text from the screenshot itself.
    assert!(
        value.get("textRecognition").is_none(),
        "local text recognition state: {value}"
    );
    assert!(
        !model
            .refs
            .iter()
            .any(|r| r.coordinate_kind.as_deref() == Some("text")),
        "text ref from a screenshot region: {value}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn clipped_canvas_regions_keep_viewport_coordinates() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<canvas width=280 height=160 style=\"position:fixed;left:-50px;top:0\"></canvas>'; scrollTo(0,0); const c=document.querySelector('canvas').getContext('2d'); c.fillStyle='white'; c.fillRect(0,0,280,160); c.strokeStyle='black'; c.lineWidth=2; c.strokeRect(80,20,20,20)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let square = model
        .refs
        .iter()
        .find(|r| r.coordinate_kind.as_deref() == Some("checkbox"))
        .expect("visible square");
    assert!(
        (28..=31).contains(&square.bounds.unwrap().x),
        "square misplaced: {square:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wide_canvas_is_scaled_with_correct_coordinate_refs() {
    let rig = Rig::new().await;
    rig.engine.viewport(1280, 720).await;
    rig.service
        .layout(Some((
            SESSION.into(),
            vw_preview::Rect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 720.0,
            },
        )))
        .await
        .unwrap();
    rig.engine.eval("document.body.innerHTML='<canvas width=1280 height=600 style=\"position:fixed;left:0;top:0\"></canvas>';scrollTo(0,0);const c=document.querySelector('canvas').getContext('2d');c.fillStyle='white';c.fillRect(0,0,1280,600);c.strokeStyle='black';c.lineWidth=2;c.strokeRect(1100,30,24,24)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let square = model
        .refs
        .iter()
        .find(|r| r.coordinate_kind.as_deref() == Some("checkbox"))
        .unwrap_or_else(|| panic!("wide canvas square: {model:?}"));
    assert!(
        (1097..=1102).contains(&square.bounds.unwrap().x),
        "scaled square misplaced: {square:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_brings_a_region_and_a_shape_in_it_into_view() {
    let rig = Rig::new().await;
    // A canvas partly below the viewport: the read lists it (and the square drawn on it) as regions.
    rig.engine.eval("document.body.style.margin='0';document.body.innerHTML='<div style=\"height:'+(innerHeight-60)+'px\"></div><canvas width=280 height=160></canvas><div style=\"height:2400px\"></div>';scrollTo(0,0);const c=document.querySelector('canvas').getContext('2d');c.fillStyle='white';c.fillRect(0,0,280,160);c.strokeStyle='black';c.lineWidth=2;c.strokeRect(20,20,20,20)").await;
    // The canvas itself, then a shape found on it, each straight after a read.
    for kind in ["region", "checkbox"] {
        rig.engine.eval("scrollTo(0,0)").await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let reference = model
            .refs
            .iter()
            .find(|r| r.coordinate_kind.as_deref() == Some(kind))
            .unwrap_or_else(|| panic!("no {kind} ref: {model:?}"))
            .id
            .clone();
        let scrolled = rig
            .browser
            .act(
                SESSION,
                vw_browser::Action::Scroll {
                    reference: Some(reference.clone()),
                    up: false,
                    amount: None,
                },
                Actor::Agent,
                &NoNavigation,
            )
            .await;
        assert!(
            scrolled.is_ok(),
            "scroll {{ref}} to {reference}: {scrolled:?}"
        );
        let inside = rig
            .engine
            .eval("(() => { const r = document.querySelector('canvas').getBoundingClientRect(); return [r.top >= 0 && r.bottom <= innerHeight, scrollY]; })()")
            .await;
        assert!(
            inside[0] == true && inside[1].as_f64().unwrap_or(0.0) > 0.0,
            "the canvas is not brought into view by {reference}: {inside}"
        );
    }
}
