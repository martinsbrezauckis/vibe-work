//! Agent input on a macOS or Linux native select is refused before it can open a popup.
//! Depends on the production act script and real Chromium control refs.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::ReadOptions;

// PREDICT: a native click on a select returns a point and a plain ArrowDown is prepared for the
// native keyboard, so the agent can open a popup that later undoes its selected value.
#[tokio::test]
async fn agent_click_and_key_refuse_before_native_select_popup() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<label>Fruit <select id=fruit><option>Apple</option><option>Cherry</option></select></label>'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model
        .refs
        .iter()
        .find(|row| row.tag == "select")
        .unwrap()
        .id;
    let page = rig.service.page_accessor(SESSION).unwrap();
    for action in ["click", "press"] {
        let request = json!({"page":page,"action":action,"ref":reference,"key":"ArrowDown","nativeKeyboard":action=="press","nativeClick":action=="click","nativeSelectPopup":true,"nativeUser":false});
        let mut answer = Value::Null;
        for _ in 0..3 {
            let raw = rig
                .service
                .evaluate(SESSION, vw_browser::scripts::act(&request))
                .await
                .unwrap();
            let payload: String = serde_json::from_str(&raw).unwrap();
            answer = serde_json::from_str(&payload).unwrap();
            if answer["status"] != "notActionable" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
        assert_eq!(answer["status"], "nativeSelectPopup", "{action}: {answer}");
        assert_eq!(
            rig.engine
                .eval("document.querySelector('#fruit').value")
                .await,
            "Apple"
        );
    }
}

/// Never navigates: the clicks below must be refused before any page effect.
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

// PREDICT: the observed node is a clickable div, so the act script and the Rust check pass it; the final point
// guard accepts the inserted select as a descendant of that div, the native press reaches the select, and the
// coordinate click answers without the native-select refusal. An ordinary ref click here is the act script's own
// click on the observed div (a native ref click, used for wider sign-in consent, goes through the same final point
// guard as the coordinate click), so it must never reach the select either.
#[tokio::test(flavor = "multi_thread")]
async fn a_select_inserted_inside_an_observed_container_is_refused_at_the_final_point() {
    for coordinates in [true, false] {
        let rig = Rig::new_in_world(true).await;
        rig.engine
            .eval(r#"
              document.body.innerHTML='<div id=box role=button tabindex=0 aria-label="Choose" style="position:fixed;left:20px;top:20px;width:200px;height:40px">Choose</div>';
              scrollTo(0,0); window.effects=[];
              for(const type of ['pointerdown','mousedown','mouseup','click'])
                window.addEventListener(type,e=>{if(e.target.closest('select'))window.effects.push(type)},true);
            "#)
            .await;
        rig.engine
            .frame_page_values(
                "new Promise(resolve=>setTimeout(resolve,Math.max(0,350-performance.now())))",
            )
            .await;
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        let reference = model
            .refs
            .iter()
            .find(|row| row.name == "Choose")
            .unwrap()
            .id
            .clone();
        // The page fills the observed container with a full-size select after the read; its geometry is unchanged.
        rig.engine
            .eval("document.querySelector('#box').insertAdjacentHTML('beforeend','<select id=fruit style=\"position:absolute;left:0;top:0;width:100%;height:100%\"><option>Apple</option><option>Cherry</option></select>')")
            .await;
        let action = if coordinates {
            json!({"action":"click","ref":reference,"x":80,"y":40})
        } else {
            json!({"action":"click","ref":reference})
        };
        let result = rig
            .browser
            .act_with_sending(
                SESSION,
                vw_browser::parse(&action).unwrap(),
                vw_browser::Actor::Agent,
                &NoNavigation,
                false,
            )
            .await;
        let native = rig
            .engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed);
        if coordinates {
            let error = result
                .as_ref()
                .expect_err("the select under the observed div was clicked");
            assert!(
                error.text.contains(
                    "Not run: this would open a dropdown list that only the user can use."
                ),
                "{error:?}"
            );
            assert_eq!(native, 0, "no native press was sent");
        }
        assert_eq!(
            rig.engine.eval("window.effects").await,
            json!([]),
            "coordinates {coordinates}: {result:?}"
        );
        assert_eq!(
            rig.engine
                .eval("document.querySelector('#fruit').value")
                .await,
            "Apple"
        );
    }
}
