//! HTML drag-and-drop acts use observed refs, honor cancellation, and transfer one shared `DataTransfer`.
//! Depends on the real browser action loop and Chromium fixture; no filesystem files are supplied.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, NavFuture, Navigate, ReadOptions};
struct NoNavigation;
impl Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn drag(accept: bool) -> (Rig, Result<serde_json::Value, vw_browser::ActError>) {
    let rig = Rig::new().await;
    rig.engine.eval(r#"(() => {
      document.body.innerHTML = '<button draggable="true" id="source">Source</button><button id="target">Destination</button>';
      window.dropValue = null;
      document.querySelector('#source').ondragstart = e => e.dataTransfer.setData('text/plain','moved');
      document.querySelector('#target').ondrop = e => { e.preventDefault(); window.dropValue = e.dataTransfer.getData('text/plain'); };
      scrollTo(0,0);
    })()"#).await;
    if accept {
        rig.engine
            .eval("document.querySelector('#target').ondragover = e => e.preventDefault()")
            .await;
    }
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Source").unwrap().id;
    let to = &model
        .refs
        .iter()
        .find(|r| r.name == "Destination")
        .unwrap()
        .id;
    let action = vw_browser::parse(&json!({"action":"drag", "ref":reference, "to":to})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    (rig, result)
}
#[tokio::test]
async fn accepted_drop_receives_dragstart_data() {
    let (rig, result) = drag(true).await;
    result.unwrap();
    assert_eq!(rig.engine.eval("window.dropValue").await, "moved");
}
#[tokio::test]
async fn unaccepted_dragover_never_dispatches_drop() {
    let (rig, result) = drag(false).await;
    assert!(result.is_err());
    assert!(rig.engine.eval("window.dropValue").await.is_null());
}
