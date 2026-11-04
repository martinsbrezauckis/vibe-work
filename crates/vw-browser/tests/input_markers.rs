//! Browser feedback regression using a real Chromium page and a host marker recorder.
//! The normal driver supplies the point; drawing must never add DOM nodes or page input.
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

#[tokio::test]
async fn typing_feedback_follows_the_build_feature_without_changing_the_page() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<label style=\"position:absolute;top:500px;left:20px\">Message<input id=message></label>';window.extraEvents=0;document.addEventListener('pointerdown',()=>extraEvents++);").await;
    let nodes = rig
        .engine
        .eval("document.querySelectorAll('*').length")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model.refs.iter().find(|r| r.name == "Message").unwrap();
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":field.id,"text":"Hello"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    let markers = rig.engine.input_markers.lock().unwrap().clone();
    assert_eq!(
        markers.len(),
        usize::from(cfg!(feature = "click-marker")),
        "typing feedback must follow the build feature"
    );
    let state = rig.engine.eval("JSON.stringify({nodes:document.querySelectorAll('*').length,events:extraEvents,value:document.querySelector('input').value,box:(()=>{let r=document.querySelector('input').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()})").await;
    let state: serde_json::Value = serde_json::from_str(state.as_str().unwrap()).unwrap();
    assert_eq!(state["events"], 0);
    assert_eq!(state["value"], "Hello");
    assert_eq!(state["nodes"], nodes);
    #[cfg(feature = "click-marker")]
    assert!((markers[0].x - state["box"][0].as_f64().unwrap()).abs() < 1.0);
    #[cfg(feature = "click-marker")]
    assert!((markers[0].y - state["box"][1].as_f64().unwrap()).abs() < 1.0);
}
