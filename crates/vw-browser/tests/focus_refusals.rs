//! Native input while the app's window is not in front: the agent gets ONE plain "bring it forward" answer for
//! keyboard (`type`, `insert_text`) and pointer (`hover`) actions, nothing is sent, and the same action runs once
//! the window is in front. (A click that carries an authorization token instead waits once for focus; see
//! `send_permissions`.) The fake host engine reports the OS adapters' shared not-in-front outcome as
//! `EngineError::FocusRequired`, the same answer the native engine gives on every OS.
//!
//! PREDICT: the hover half fails before the fix: hover answered the click's focus-wait text ("... this click will
//! try once"), which names a click and promises a retry that hover never gets, while `type` / `insert_text`
//! answered the keyboard text. After the fix all three answer the same text, and it never says "click".
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

const NOT_IN_FRONT_REFUSAL: &str = "No input was sent because the browser window is not in front. Ask the user to bring the pane forward, then read the page before trying again.";

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn page() -> (Rig, String, String) {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<label>Message<input id=message value=abcd></label><button id=menu style=\"position:fixed;left:20px;top:80px;width:100px;height:40px\">Menu</button>';window.keys=0;window.overs=0;document.addEventListener('keydown',()=>keys++);menu.addEventListener('mouseover',()=>overs++);scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let id = |name: &str| {
        model
            .refs
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .id
            .clone()
    };
    let (field, menu) = (id("Message"), id("Menu"));
    (rig, field, menu)
}

async fn act(rig: &Rig, request: Value) -> Result<Value, vw_browser::ActError> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn keyboard_input_while_the_window_is_not_in_front_gets_the_plain_refusal() {
    let (rig, field, _) = page().await;
    rig.engine.focus_required.store(true, Ordering::Relaxed);
    for request in [
        json!({"action":"type","clear":false,"ref":field,"text":"!"}),
        json!({"action":"insert_text","ref":field,"text":"!"}),
    ] {
        let error = act(&rig, request.clone()).await.unwrap_err();
        assert_eq!(error.text, NOT_IN_FRONT_REFUSAL, "{request}");
    }
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
    assert_eq!(rig.engine.eval("message.value").await, "abcd");
    assert_eq!(rig.engine.eval("keys").await, 0);
    // Brought forward: the same action runs.
    rig.engine.focus_required.store(false, Ordering::Relaxed);
    act(
        &rig,
        json!({"action":"type","clear":false,"ref":field,"text":"!"}),
    )
    .await
    .unwrap();
    assert_eq!(rig.engine.eval("message.value.length").await, 5);
    assert_eq!(rig.engine.eval("message.value.includes('!')").await, true);
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_while_the_window_is_not_in_front_gets_the_plain_refusal_and_runs_when_brought_forward()
 {
    let (rig, _, menu) = page().await;
    rig.engine.focus_required.store(true, Ordering::Relaxed);
    let error = act(&rig, json!({"action":"hover","ref":menu}))
        .await
        .unwrap_err();
    assert_eq!(error.text, NOT_IN_FRONT_REFUSAL);
    assert!(!error.text.contains("click"), "{}", error.text);
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
    assert_eq!(rig.engine.eval("overs").await, 0);
    // Brought forward: the same action runs.
    rig.engine.focus_required.store(false, Ordering::Relaxed);
    act(&rig, json!({"action":"hover","ref":menu}))
        .await
        .unwrap();
    assert!(rig.engine.eval("overs").await.as_u64().unwrap() > 0);
}
