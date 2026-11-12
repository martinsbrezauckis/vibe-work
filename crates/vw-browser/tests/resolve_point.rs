//! Read-only observed point resolution shares the click identity guard and refuses covering/replaced nodes.
//! Uses the production driver with an isolated Chromium fixture; no input should be delivered.
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
async fn point_resolution_returns_the_checked_target_without_input() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<button id=t style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Resolve me</button>';scrollTo(0,0);window.inputs=0;document.addEventListener('mousedown',()=>inputs++)").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let action = vw_browser::parse(&json!({"action":"resolve_point","x":50,"y":40})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action.clone(), Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        result["untrustedPageContent"]["target"]["name"],
        "Resolve me"
    );
    assert_eq!(result["result"]["checked"], true);
    assert_eq!(rig.engine.eval("inputs").await, 0);
    rig.engine.eval("const c=document.createElement('div');c.style='position:fixed;inset:0;background:white;z-index:9999';document.body.append(c)").await;
    assert!(
        rig.browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .is_err()
    );
    assert_eq!(rig.engine.eval("inputs").await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn child_point_resolution_checks_its_ancestor_geometry() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<iframe id=f style=\"position:fixed;left:10px;top:10px;width:220px;height:140px;border:0\"></iframe>';scrollTo(0,0);new Promise(resolve=>{f.onload=resolve;f.srcdoc='<button style=\"position:fixed;left:10px;top:10px;width:100px;height:40px\">Child target</button>'})").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model
        .refs
        .iter()
        .find(|r| r.name == "Child target")
        .unwrap();
    let bounds = target.bounds.unwrap();
    let action =
        vw_browser::parse(&json!({"action":"resolve_point","x":bounds.x+10,"y":bounds.y+10}))
            .unwrap();
    let result = rig
        .browser
        .act(SESSION, action.clone(), Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        result["untrustedPageContent"]["target"]["name"],
        "Child target"
    );
    rig.engine.eval("f.style.left='40px'").await;
    assert!(
        rig.browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .is_err()
    );
}
