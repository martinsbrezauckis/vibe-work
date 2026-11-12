//! Observed-point pointer motion uses native hover without button input, including canvas and SVG.
//! Uses the browser fixture and production point binding; target replacements must refuse.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn pointer_moves_to_observed_coordinates_without_pressing_a_button() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<button style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Move here</button>';scrollTo(0,0);window.events=[];for(const name of ['pointermove','mousedown','click'])document.addEventListener(name,e=>events.push({type:e.type,trusted:e.isTrusted}))").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let action = vw_browser::parse(&json!({"action":"move","x":50,"y":40})).unwrap();
    rig.browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    let events = rig.engine.eval("events").await;
    let events = events.as_array().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "pointermove" && e["trusted"] == true)
    );
    assert!(
        !events
            .iter()
            .any(|e| e["type"] == "mousedown" || e["type"] == "click")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn canvas_and_svg_points_move_natively_and_replaced_nodes_refuse() {
    for markup in [
        "<canvas width=200 height=120></canvas>",
        "<svg width=200 height=120><rect role=button aria-label=Shape width=200 height=120 fill=blue /></svg>",
    ] {
        let rig = Rig::new_in_world(true).await;
        rig.engine.eval(&format!("document.body.innerHTML={};document.body.style.margin='0';scrollTo(0,0);window.moves=0;document.addEventListener('pointermove',e=>{{if(e.isTrusted)moves++}})",json!(markup))).await;
        rig.browser
            .read(SESSION, ReadOptions::default(), true, Actor::User)
            .await
            .unwrap();
        let action = vw_browser::parse(&json!({"action":"move","x":40,"y":40})).unwrap();
        rig.browser
            .act(SESSION, action, Actor::User, &NoNavigation)
            .await
            .unwrap();
        assert!(rig.engine.eval("moves").await.as_u64().unwrap() > 0);
        rig.browser
            .read(SESSION, ReadOptions::default(), true, Actor::User)
            .await
            .unwrap();
        rig.engine.eval("document.body.firstElementChild.replaceWith(document.body.firstElementChild.cloneNode(true));moves=0").await;
        let action = vw_browser::parse(&json!({"action":"move","x":40,"y":40})).unwrap();
        assert!(
            rig.browser
                .act(SESSION, action, Actor::User, &NoNavigation)
                .await
                .is_err()
        );
        assert_eq!(rig.engine.eval("moves").await, 0);
    }
}
