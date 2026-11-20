//! HTML drag-and-drop acts use observed refs, honor cancellation, and transfer one shared `DataTransfer`.
//! Depends on the real browser action loop and Chromium fixture; no filesystem files are supplied.
//! PREDICT: a selector read gives a draggable plain element a ref, but the act lookup loses it before input.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, NavFuture, Navigate, ReadOptions, Selector, SelectorKind};
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

#[tokio::test]
async fn selector_drag_retains_a_plain_draggable_source_until_input() {
    let rig = Rig::new().await;
    rig.engine.eval(r#"document.body.innerHTML = '<div id="source" draggable="true" style="position:absolute;top:60px;left:20px;width:90px;height:30px">Drag item</div><form method="post"><button id="target" style="position:absolute;top:120px;left:20px;width:90px;height:30px">Send POST</button></form>';
        window.drops = 0; window.dragStarts = 0; window.submissions = 0;
        document.querySelector('form').onsubmit = e => { e.preventDefault(); window.submissions++; };
        document.querySelector('#source').ondragstart = e => { window.dragStarts++; e.dataTransfer.setData('text/plain', 'kept'); };
        document.querySelector('#target').ondragover = e => e.preventDefault();
        document.querySelector('#target').ondrop = e => { e.preventDefault(); if (e.dataTransfer.getData('text/plain') === 'kept') { window.drops++; e.target.form.requestSubmit(); } };
        scrollTo(0,0)"#).await;
    let (read, _) = rig
        .browser
        .read(
            SESSION,
            ReadOptions {
                selector: Some(Selector {
                    kind: SelectorKind::Css,
                    value: "#source,#target".into(),
                    index: None,
                }),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap();
    let refs = read["untrustedPageContent"]["refs"].as_array().unwrap();
    let source = refs.iter().find(|r| r["name"] == "Drag item").unwrap()["ref"]
        .as_str()
        .unwrap();
    let target = refs.iter().find(|r| r["name"] == "Send POST").unwrap()["ref"]
        .as_str()
        .unwrap();
    let action = vw_browser::parse(&json!({"action":"drag", "ref":source, "to":target})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    assert!(result.is_ok(), "a fresh selector drag must run: {result:?}");
    assert_eq!(result.unwrap()["ok"], true);
    assert_eq!(
        rig.engine
            .eval("[window.dragStarts, window.drops, window.submissions]")
            .await,
        json!([1, 1, 1])
    );
}
