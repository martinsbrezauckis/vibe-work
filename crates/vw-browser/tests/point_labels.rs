//! Checked label clicks authorize only the original associated control and return a confirmed receipt.
//! Uses real Chromium and isolated production scripts; binding changes cannot activate another control.
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

async fn label_click(wrapping: bool, mutate: &str) {
    let rig = Rig::new_in_world(true).await;
    let control = "<button id=control type=submit>Send</button>";
    let label = if wrapping {
        format!("<label id=target role=button aria-label='Label action'>Label{control}</label>")
    } else {
        format!(
            "<label id=target role=button aria-label='Label action' for=control>Label</label>{control}"
        )
    };
    rig.engine.eval(&format!(
        "document.body.innerHTML={};scrollTo(0,0);window.effects=[];document.querySelector('form').onsubmit=e=>{{e.preventDefault();window.effects.push(e.submitter.id)}};",
        json!(format!("<style>#target{{position:fixed;left:20px;top:20px;width:100px;height:40px}}button{{position:fixed;left:180px;top:100px}}</style><form method=post>{label}<button id=other>Other</button></form>"))
    )).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = &model
        .refs
        .iter()
        .find(|r| r.name == "Label action")
        .unwrap()
        .id;
    if !mutate.is_empty() {
        rig.engine.eval(&format!("document.querySelector('#target').addEventListener('click',()=>{{{mutate}}},{{once:true}})")).await;
    }
    let result = rig
        .browser
        .act_with_sending(
            SESSION,
            vw_browser::parse(&json!({"action":"click","ref":target,"x":40,"y":40})).unwrap(),
            Actor::Agent,
            &NoNavigation,
            true,
        )
        .await;
    let effects = rig.engine.eval("window.effects").await;
    if mutate.is_empty() {
        assert!(result.is_ok(), "{result:?}; {effects}");
        assert_eq!(effects, json!(["control"]));
    } else {
        assert_eq!(
            effects,
            json!([]),
            "a changed association must have no form effect: {result:?}"
        );
        assert!(result.unwrap_err().text.contains("the page moved"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_label_forwarding_confirms_the_validated_control() {
    for wrapping in [false, true] {
        label_click(wrapping, "").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn label_reassociation_during_activation_cannot_submit_another_control() {
    label_click(false, "document.querySelector('#target').htmlFor='other'").await;
    label_click(
        true,
        "document.querySelector('#control').replaceWith(document.querySelector('#other'))",
    )
    .await;
}
