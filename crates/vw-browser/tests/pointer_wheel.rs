//! Native wheel input stays on the observed nested pane and proves the trusted event and actual offsets.
//! Uses the browser fixture with production point guards; stale/covered targets must receive no input.
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
async fn native_wheel_scrolls_only_the_observed_pane_and_reports_its_real_offset() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<div id=pane role=button tabindex=0 aria-label=Pane style=\"position:fixed;left:10px;top:10px;width:180px;height:120px;overflow:auto\"><div style=\"width:900px;height:900px\">Wide list</div></div>';scrollTo(0,0);window.wheels=[];window.downs=0;document.addEventListener('wheel',e=>wheels.push({trusted:e.isTrusted,x:e.deltaX,y:e.deltaY}),{passive:true});document.addEventListener('mousedown',()=>downs++)").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let action =
        vw_browser::parse(&json!({"action":"wheel","x":50,"y":50,"deltaX":80,"deltaY":120}))
            .unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("wheels").await,
        json!([{"trusted":true,"x":80,"y":120}])
    );
    assert_eq!(
        rig.engine
            .eval("[pane.scrollLeft,pane.scrollTop,scrollX,scrollY,downs]")
            .await,
        json!([80, 120, 0, 0, 0])
    );
    assert_eq!(result["result"]["scroll"]["x"], 80, "{result}");
    assert_eq!(result["result"]["scroll"]["y"], 120, "{result}");
    assert_eq!(result["Outcome"], "the intended scroll container moved");
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Pane").unwrap().id;
    let action =
        vw_browser::parse(&json!({"action":"wheel","ref":reference,"deltaY":-60})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(result["result"]["scroll"]["y"], 60, "{result}");
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    rig.engine.eval("document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;inset:0;width:300px;height:200px\">Cover</button>');wheels=[]").await;
    let action = vw_browser::parse(&json!({"action":"wheel","x":50,"y":50,"deltaY":100})).unwrap();
    assert!(
        rig.browser
            .act(SESSION, action, Actor::User, &NoNavigation)
            .await
            .is_err()
    );
    assert_eq!(rig.engine.eval("wheels.length").await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cover_inserted_by_native_motion_cannot_receive_wheel_input() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<button style=\"position:fixed;left:10px;top:10px;width:150px;height:80px\">Target</button>';scrollTo(0,0);window.wheels=0;window.addEventListener('wheel',()=>wheels++,true);window.addEventListener('mousemove',()=>document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;inset:0;width:300px;height:200px\">Cover</button>'),{once:true})").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    rig.engine
        .move_before_click
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let action = vw_browser::parse(&json!({"action":"wheel","x":50,"y":50,"deltaY":100})).unwrap();
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert!(error.text.contains("blocked native input"), "{error:?}");
    assert_eq!(rig.engine.eval("wheels").await, 0);
}
