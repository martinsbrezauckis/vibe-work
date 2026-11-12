//! A checked click is refused while the person is clicking, and is never confirmed by the person's own click.
//! Uses production isolated scripts, independent Chromium input as the person's pointer, and the shared engine.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::atomic::Ordering;
use std::time::Duration;
use support::{Rig, SESSION};
use vw_browser::{Actor, Outcome, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

const BUSY: &str = "the user is using the browser right now; try again in a moment";
const PERSON: &str = "the user clicked in the browser at the same time, so earlier refs may no longer match; read the page again";

async fn prepared() -> (Rig, vw_browser::Action) {
    let rig = Rig::new_in_world(true).await;
    // These cases exercise existing documents; new-document input expiry has separate coverage.
    rig.engine
        .eval("new Promise(resolve=>setTimeout(resolve,Math.max(0,350-performance.now())))")
        .await;
    rig.engine.eval(r#"document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">A</button>';
      scrollTo(0,0);window.presses=[];
      window.addEventListener('mousedown',e=>{if(e.isTrusted)window.presses.push(e.target.id||e.target.tagName)},true)"#).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    (rig, action)
}

async fn act(
    rig: &Rig,
    action: vw_browser::Action,
) -> Result<serde_json::Value, vw_browser::ActError> {
    rig.browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
}

fn refused_before_input(rig: &Rig, result: &Result<serde_json::Value, vw_browser::ActError>) {
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "{result:?}"
    );
    let error = result
        .as_ref()
        .expect_err("checked input ran while the user was clicking");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(error.text.contains(BUSY), "{error:?}");
    assert!(!error.text.contains("native input was sent"), "{error:?}");
}

fn not_confirmed(rig: &Rig, result: &Result<serde_json::Value, vw_browser::ActError>) {
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        1,
        "{result:?}"
    );
    let error = result
        .as_ref()
        .expect_err("the user's click confirmed the agent's input");
    assert_eq!(error.outcome, Outcome::Failed, "{error:?}");
    assert!(error.text.contains(PERSON), "{error:?}");
    assert!(error.text.contains("native input was sent"), "{error:?}");
    let state = rig.browser.sessions().state(SESSION);
    let entry = state.log.last().unwrap();
    assert_eq!(entry.outcome, Outcome::Failed);
}

#[tokio::test(flavor = "multi_thread")]
async fn person_click_is_the_only_sequence_after_lost_agent_input() {
    let (rig, action) = prepared().await;
    rig.engine.agent_input_lost.store(true, Ordering::Relaxed);
    rig.engine
        .person_click_after_dispatch
        .store(true, Ordering::Relaxed);
    let result = act(&rig, action).await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target"])
    );
    not_confirmed(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn person_click_after_the_agents_input_is_not_confirmed() {
    let (rig, action) = prepared().await;
    rig.engine
        .person_click_after_dispatch
        .store(true, Ordering::Relaxed);
    let result = act(&rig, action).await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target", "target"])
    );
    not_confirmed(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn person_click_after_the_mark_refuses_before_input() {
    let (rig, action) = prepared().await;
    rig.engine.agent_input_lost.store(true, Ordering::Relaxed);
    rig.engine
        .person_click_after_mark
        .store(true, Ordering::Relaxed);
    let result = act(&rig, action).await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target"])
    );
    refused_before_input(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn person_click_making_an_unregistered_frame_cannot_confirm_input() {
    let (rig, action) = prepared().await;
    // The person's click covers A with a frame the host has not registered yet (macOS registers frames
    // asynchronously); the agent's input would land in that frame.
    rig.engine.eval(r#"window.coverReady=false;
      document.querySelector('#target').addEventListener('click',e=>{
        if(!e.isTrusted)return;
        const f=document.createElement('iframe');f.style='position:fixed;left:20px;top:20px;width:100px;height:40px;border:0;z-index:100';
        f.onload=()=>{f.contentWindow.addEventListener('mousedown',e=>{if(e.isTrusted)window.presses.push(e.target.id)},true);window.coverReady=true};
        f.srcdoc='<button id="cover" style="position:fixed;inset:0;width:100%;height:100%">B</button>';document.body.append(f);
      },{once:true})"#).await;
    rig.engine.registry_lag.store(true, Ordering::Relaxed);
    rig.engine
        .person_click_after_mark
        .store(true, Ordering::Relaxed);
    let result = act(&rig, action).await;
    assert!(
        result.is_err(),
        "the user's click confirmed input in a new frame: {result:?}"
    );
    let presses = rig.engine.eval("window.presses").await;
    assert_eq!(presses[0], "target", "{presses}");
    refused_before_input(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn recent_person_press_refuses_before_input() {
    let (rig, action) = prepared().await;
    // Elsewhere on the page, just before the agent's click.
    rig.engine.person_click(200.0, 150.0).await;
    let result = act(&rig, action).await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["BODY"])
    );
    refused_before_input(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn held_person_pointer_refuses_before_input() {
    let (rig, action) = prepared().await;
    rig.engine.person_holding.store(true, Ordering::Relaxed);
    let result = act(&rig, action).await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!([])
    );
    refused_before_input(&rig, &result);
}

#[tokio::test(flavor = "multi_thread")]
async fn own_input_alone_still_confirms_after_an_earlier_person_click() {
    let (rig, action) = prepared().await;
    rig.engine.person_click(200.0, 150.0).await;
    tokio::time::sleep(support::PERSON_RECENT + Duration::from_millis(200)).await;
    let receipt = act(&rig, action)
        .await
        .expect("own checked input was not confirmed");
    assert_eq!(receipt["result"]["nativeClick"], true, "{receipt}");
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["BODY", "target"])
    );
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
}
