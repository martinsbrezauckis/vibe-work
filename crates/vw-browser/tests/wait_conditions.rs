//! Bounded read-only waits through the real page driver; verifies attachment separately from visibility.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION, URL};
use vw_browser::{Actor, ReadOptions};
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn wait(rig: &Rig, mut args: Value) -> Value {
    args["action"] = json!("wait");
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
async fn attachment_and_visibility_are_distinct_and_poll_delayed_elements() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='';setTimeout(()=>{let b=document.createElement('button');b.id='later';b.hidden=true;b.textContent='Later';document.body.append(b)},120)").await;
    assert_eq!(
        wait(
            &rig,
            json!({"selector":"#later","state":"attached","timeoutMs":800})
        )
        .await["result"]["met"],
        true
    );
    assert_eq!(
        wait(
            &rig,
            json!({"selector":"#later","state":"visible","timeoutMs":50})
        )
        .await["result"]["met"],
        false
    );
    assert_eq!(
        wait(
            &rig,
            json!({"selector":"#later","state":"hidden","timeoutMs":50})
        )
        .await["result"]["met"],
        true
    );
}
#[tokio::test]
async fn removal_is_observed_without_replacing_the_reference_baseline() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<button id=target>Remove me</button>'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model
        .refs
        .iter()
        .find(|r| r.name == "Remove me")
        .unwrap()
        .id;
    assert_eq!(
        wait(
            &rig,
            json!({"ref":reference,"state":"detached","timeoutMs":50})
        )
        .await["result"]["met"],
        false
    );
    rig.engine.eval("setTimeout(()=>target.remove(),120)").await;
    let start = std::time::Instant::now();
    assert_eq!(
        wait(
            &rig,
            json!({"ref":reference,"state":"detached","timeoutMs":800})
        )
        .await["result"]["met"],
        true
    );
    assert!(start.elapsed() >= std::time::Duration::from_millis(100));
    assert_eq!(
        rig.engine
            .eval("document.getElementById('target')===null")
            .await,
        true
    );
}
#[tokio::test]
async fn address_load_and_plain_delay_report_actual_condition() {
    let rig = Rig::new().await;
    assert_eq!(
        wait(&rig, json!({"url":URL,"timeoutMs":100})).await["result"]["met"],
        true
    );
    assert_eq!(
        wait(&rig, json!({"load":"interactive","timeoutMs":100})).await["result"]["met"],
        true
    );
    assert_eq!(
        wait(&rig, json!({"url":"https://never.test/","timeoutMs":50})).await["result"]["met"],
        false
    );
    let start = std::time::Instant::now();
    assert_eq!(wait(&rig, json!({"ms":100})).await["result"]["met"], true);
    assert!(start.elapsed() >= std::time::Duration::from_millis(100));
}

#[test]
fn ambiguous_or_unbounded_conditions_are_refused_before_waiting() {
    for args in [
        json!({"action":"wait","selector":"button","url":"https://x.test/"}),
        json!({"action":"wait","selector":"button","state":"gone"}),
        json!({"action":"wait","ms":10001}),
        json!({"action":"wait","ms":100,"timeoutMs":500}),
        json!({"action":"wait","selector":"button","timeoutMs":0}),
        json!({"action":"wait","networkIdle":true}),
        json!({"action":"wait","expression":"true"}),
    ] {
        assert!(vw_browser::parse(&args).is_err(), "{args}");
    }
}
#[tokio::test]
async fn an_identical_replacement_does_not_keep_the_original_node_attached() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<button id=target>Original</button>'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Original").unwrap().id;
    rig.engine
        .eval("target.replaceWith(target.cloneNode(true))")
        .await;
    assert_eq!(
        wait(
            &rig,
            json!({"ref":reference,"state":"detached","timeoutMs":200})
        )
        .await["result"]["met"],
        true
    );
}
