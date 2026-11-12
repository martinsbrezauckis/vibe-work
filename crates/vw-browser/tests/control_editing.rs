//! Checked focus and text insertion through the production driver and a disposable browser document.
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
async fn field() -> (Rig, String) {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<label>Message<input id=message value=abcd></label>';window.keys=0;document.addEventListener('keydown',()=>keys++);").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == "Message")
        .unwrap()
        .id
        .clone();
    (rig, reference)
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
#[tokio::test]
async fn insertion_replaces_only_the_selection_without_keyboard_events() {
    let (rig, reference) = field().await;
    rig.engine
        .eval("message.focus();message.setSelectionRange(1,3)")
        .await;
    let receipt = act(
        &rig,
        json!({"action":"insert_text","ref":reference,"text":"Ω"}),
    )
    .await
    .unwrap();
    assert_eq!(rig.engine.eval("message.value").await, "aΩd");
    assert_eq!(rig.engine.eval("keys").await, 0);
    assert_eq!(receipt["untrustedPageContent"]["target"]["valueNow"], "aΩd");
}
#[tokio::test]
async fn focus_and_blur_report_the_observed_focus() {
    let (rig, reference) = field().await;
    act(&rig, json!({"action":"focus","ref":reference}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("document.activeElement===message").await,
        true
    );
    act(&rig, json!({"action":"blur","ref":reference}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("document.activeElement===message").await,
        false
    );
}
#[tokio::test]
async fn cancelled_insertion_does_not_report_success_or_repeat_input() {
    let (rig, reference) = field().await;
    rig.engine
        .eval("window.attempts=0;message.onbeforeinput=e=>{attempts++;e.preventDefault()}")
        .await;
    assert!(
        act(
            &rig,
            json!({"action":"insert_text","ref":reference,"text":"x"})
        )
        .await
        .is_err()
    );
    assert_eq!(rig.engine.eval("message.value").await, "abcd");
    assert_eq!(rig.engine.eval("attempts").await, 1);
}

#[tokio::test]
async fn append_alias_and_contenteditable_preserve_surrounding_content() {
    let (rig, reference) = field().await;
    rig.engine
        .eval("message.focus();message.setSelectionRange(4,4)")
        .await;
    act(
        &rig,
        json!({"action":"type","clear":false,"ref":reference,"text":"!"}),
    )
    .await
    .unwrap();
    assert_eq!(rig.engine.eval("message.value").await, "abcd!");
    rig.engine.eval("document.body.innerHTML='<div id=message role=textbox aria-label=Message contenteditable><b>bold</b> tail</div>';message.focus();let r=document.createRange();r.selectNodeContents(message);r.collapse(false);let s=getSelection();s.removeAllRanges();s.addRange(r)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Message").unwrap().id;
    act(
        &rig,
        json!({"action":"insert_text","ref":reference,"text":"!"}),
    )
    .await
    .unwrap();
    assert_eq!(
        rig.engine.eval("message.innerHTML").await,
        "<b>bold</b> tail!"
    );
}
#[tokio::test]
async fn insertion_refuses_a_field_that_becomes_secret_during_beforeinput() {
    for action in [
        json!({"action":"insert_text","text":"never inserted"}),
        json!({"action":"press","key":"Space"}),
    ] {
        let (rig, reference) = field().await;
        rig.engine
            .eval("message.onbeforeinput=()=>message.type='password'")
            .await;
        let mut action = action;
        action["ref"] = json!(reference);
        let error = act(&rig, action).await.unwrap_err();
        assert!(!error.text.contains("abcd"));
        assert_eq!(rig.engine.eval("message.value").await, "abcd");
    }
}

#[tokio::test]
async fn insertion_does_not_send_beforeinput_to_a_field_that_becomes_secret_on_focus() {
    let (rig, reference) = field().await;
    rig.engine.eval("window.attempts=0;message.onfocus=()=>message.type='password';message.onbeforeinput=()=>attempts++").await;
    assert!(
        act(
            &rig,
            json!({"action":"insert_text","ref":reference,"text":"private input"})
        )
        .await
        .is_err()
    );
    assert_eq!(rig.engine.eval("attempts").await, 0);
    assert_eq!(rig.engine.eval("message.value").await, "abcd");
}

#[tokio::test]
async fn highlight_points_at_the_control_without_focus_or_input_and_clears() {
    let (rig, reference) = field().await;
    let receipt = act(&rig, json!({"action":"highlight","ref":reference}))
        .await
        .unwrap();
    assert_eq!(receipt["result"]["effect"], "highlighted");
    assert_eq!(
        rig.engine.eval("document.activeElement===message").await,
        false
    );
    assert_eq!(rig.engine.eval("keys").await, 0);
    assert_eq!(
        rig.engine
            .eval("document.querySelector('[data-vw-highlight]')?.style.pointerEvents")
            .await,
        "none"
    );
    act(
        &rig,
        json!({"action":"highlight","ref":reference,"clear":true}),
    )
    .await
    .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('[data-vw-highlight]')===null")
            .await,
        true
    );
}
