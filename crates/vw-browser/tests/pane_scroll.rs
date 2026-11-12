//! Scroll exactly the requested pane and read its real offsets; exercised through the production browser driver.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn act(rig: &Rig, args: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn scrolling_a_pane_moves_both_axes_without_scrolling_the_page() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<div id=pane tabindex=0 aria-label=Pane style=\"width:180px;height:100px;overflow:auto\"><div style=\"width:1200px;height:1600px\">Content</div></div><div style=height:3000px>Page</div>'").await;
    rig.engine.eval("scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Pane").unwrap().id;
    let result = act(
        &rig,
        json!({"action":"scroll","ref":reference,"x":70,"y":120}),
    )
    .await;
    assert_eq!(
        rig.engine
            .eval("[pane.scrollLeft,pane.scrollTop,scrollY]")
            .await,
        json!([70, 120, 0])
    );
    assert_eq!(result["result"]["scroll"]["x"], 70);
    assert_eq!(result["result"]["scroll"]["y"], 120);
    assert_eq!(result["result"]["scroll"]["moved"], true);
    let read = act(&rig, json!({"action":"scroll_position","ref":reference})).await;
    assert_eq!(read["result"]["scroll"]["x"], 70);
    assert_eq!(read["result"]["scroll"]["y"], 120);
    assert_eq!(
        rig.engine.eval("[pane.scrollLeft,pane.scrollTop]").await,
        json!([70, 120])
    );
}
#[tokio::test]
async fn horizontal_document_scroll_and_edges_are_observed() {
    let rig = Rig::new().await;
    rig.engine
        .eval(
            "document.body.innerHTML='<div style=\"width:2000px;height:100px\">Wide content</div>'",
        )
        .await;
    rig.engine.eval("scrollTo(0,0)").await;
    let first = act(
        &rig,
        json!({"action":"scroll","direction":"right","amount":200}),
    )
    .await;
    assert_eq!(rig.engine.eval("scrollX").await, 200);
    assert_eq!(first["result"]["scroll"]["x"], 200);
    act(
        &rig,
        json!({"action":"scroll","direction":"right","amount":20000}),
    )
    .await;
    let end = act(
        &rig,
        json!({"action":"scroll","direction":"right","amount":200}),
    )
    .await;
    assert_eq!(end["result"]["scroll"]["moved"], false);
    assert_eq!(end["result"]["scroll"]["endReached"], true);
}
