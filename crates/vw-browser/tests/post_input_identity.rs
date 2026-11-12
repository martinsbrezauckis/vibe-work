//! Final control-state proof follows the observed node after input changes its DOM path.
//! Production browser scripts in an isolated page fixture; replacements never inherit verification authority.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, Outcome, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn page(script: &str) -> Rig {
    let rig = Rig::new().await;
    rig.engine.eval("document.head.innerHTML=''; document.body.style=''; document.body.innerHTML='<main><section><label>Date <input id=entry value=03/14/2026></label></section></main>';window.inputs=0;document.querySelector('#entry').addEventListener('input',()=>window.inputs++);scrollTo(0,0)").await;
    rig.engine.eval(script).await;
    rig
}

async fn type_date(rig: &Rig) -> Result<Value, vw_browser::ActError> {
    let (read, _) = rig
        .browser
        .read(
            SESSION,
            ReadOptions {
                find: Some("Date".into()),
                ..ReadOptions::default()
            },
            false,
            Actor::Agent,
        )
        .await
        .unwrap();
    let reference = read["untrustedPageContent"]["refs"][0]["ref"].clone();
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"type","ref":reference,"text":"04/15/2026"}))
                .unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

#[tokio::test]
async fn opening_a_calendar_keeps_the_same_nodes_successful_value_and_sends_input_once() {
    let rig = page("document.querySelector('#entry').onfocus=()=>{const calendar=document.createElement('section');calendar.innerHTML='<button>Calendar choice</button>';document.querySelector('main').append(calendar)}").await;
    let result = type_date(&rig).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#entry').value")
            .await,
        "04/15/2026"
    );
    assert_eq!(rig.engine.eval("window.inputs").await, 1);
    let receipt = result.expect("the same node kept the requested date after its DOM path changed");
    assert_eq!(
        receipt["untrustedPageContent"]["target"]["valueNow"],
        "04/15/2026"
    );
}

#[tokio::test]
async fn an_identical_replacement_after_input_cannot_supply_the_original_nodes_proof() {
    let rig = page("document.querySelector('#entry').addEventListener('input',e=>{const clone=e.target.cloneNode(true);clone.value=e.target.value;e.target.replaceWith(clone)})").await;
    let result = type_date(&rig).await;
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#entry').value")
            .await,
        "04/15/2026"
    );
    assert_eq!(rig.engine.eval("window.inputs").await, 1);
    let error = result.expect_err("a replacement cannot attest the original observed control");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("Not done:"), "{error:?}");
    assert!(error.text.contains("do not repeat"), "{error:?}");
}
