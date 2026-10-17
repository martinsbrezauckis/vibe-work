//! Browser actions prove the requested state on real controls before reporting success.
//! Depends on support's Chromium page and the production observe/act/receipt loop; called by cargo test.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Action, Actor, NavFuture, Navigate, Outcome, ReadOptions};

struct NoNavigation;
impl Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn controls() -> Rig {
    let rig = Rig::new().await;
    install_controls(&rig).await;
    rig
}

async fn install_controls(rig: &Rig) {
    rig.engine.eval(r#"(() => {
      document.head.innerHTML = '<title>Controls</title>';
      document.body.style = '';
      document.body.innerHTML = `<form onsubmit="event.preventDefault(); window.submissions=(window.submissions||0)+1">
        <label><input id="normal" type="checkbox">Normal box</label>
        <label><input id="blocked" type="checkbox">Blocked box</label>
        <label><input id="revert" type="checkbox">Reverted box</label>
        <label><input id="later" type="checkbox">Later box</label>
        <label><input id="radio" type="radio" checked>Chosen radio</label>
        <label><input id="radioBlocked" type="radio">Blocked radio</label>
        <button type="button" role="switch" id="switch" aria-checked="false">Reverted switch</button>
        <button type="button" role="checkbox" id="accept" aria-checked="false">Later accepted box</button>
        <label for="select">Choice</label><select id="select"><option value="a">Alpha</option><option value="b">Beta</option></select>
        <label for="selectRevert">Reverted choice</label><select id="selectRevert"><option value="a">Alpha</option><option value="b">Beta</option></select>
        <label for="selectLater">Later choice</label><select id="selectLater"><option value="a">Alpha</option><option value="b">Beta</option></select>
        <label>Text<input id="text"></label><label>Reverted text<input id="textRevert"></label>
        <label>Later text<input id="textLater"></label><label>Blocked text<input id="textBlocked"></label>
        <div contenteditable="true" role="textbox" aria-label="Reverted editor" id="editor"></div>
      </form>`;
      const byId = id => document.getElementById(id);
      for (const id of ['blocked', 'radioBlocked']) byId(id).addEventListener('click', e => e.preventDefault());
      byId('revert').addEventListener('change', e => { e.target.checked=false; });
      byId('later').addEventListener('change', e => setTimeout(() => { e.target.checked=false; }, 60));
      byId('switch').addEventListener('click', e => { e.target.setAttribute('aria-checked','true'); queueMicrotask(() => e.target.setAttribute('aria-checked','false')); });
      byId('accept').addEventListener('click', e => setTimeout(() => e.target.setAttribute('aria-checked','true'), 60));
      byId('selectRevert').addEventListener('change', e => { e.target.value='a'; });
      byId('selectLater').addEventListener('change', e => setTimeout(() => { e.target.value='a'; }, 60));
      byId('textRevert').addEventListener('input', e => { e.target.value=''; });
      byId('textLater').addEventListener('input', e => setTimeout(() => { e.target.value=''; }, 60));
      byId('textBlocked').addEventListener('beforeinput', e => e.preventDefault());
      byId('editor').addEventListener('input', e => { e.target.textContent=''; });
      scrollTo(0,0);
    })()"#).await;
}

async fn act(rig: &Rig, name: &str, mut request: Value) -> Result<Value, vw_browser::ActError> {
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    request["ref"] = json!(model.refs.iter().find(|r| r.name == name).expect(name).id);
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
async fn prevented_checkbox_click_is_an_explicit_unmet_result() {
    let rig = controls().await;
    let error = act(
        &rig,
        "Blocked box",
        json!({"action":"check", "checked":true}),
    )
    .await
    .expect_err("preventDefault must not report done");
    assert_eq!(error.outcome, Outcome::Failed);
    assert_eq!(
        error.text,
        "The box is still unchecked; the page did not accept the change."
    );
    assert_eq!(
        rig.browser
            .sessions()
            .state(SESSION)
            .log
            .last()
            .unwrap()
            .outcome,
        Outcome::Failed
    );
}

#[tokio::test]
async fn synchronous_and_delayed_checkbox_reverts_are_unmet() {
    let rig = controls().await;
    for name in ["Reverted box", "Later box", "Reverted switch"] {
        let error = act(&rig, name, json!({"action":"check", "checked":true}))
            .await
            .expect_err(name);
        assert_eq!(error.outcome, Outcome::Failed);
        assert!(
            error.text.contains("did not accept the change"),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn delayed_checkbox_revert_is_unmet_after_settling() {
    let rig = controls().await;
    let error = act(&rig, "Later box", json!({"action":"check", "checked":true}))
        .await
        .expect_err("a delayed revert must not report done");
    assert_eq!(error.outcome, Outcome::Failed);
    assert_eq!(
        error.text,
        "The box is still unchecked; the page did not accept the change."
    );
}

#[tokio::test]
async fn delayed_checkbox_acceptance_is_proved_without_repeating_the_click() {
    let rig = controls().await;
    rig.engine.eval("window.clicks=0; document.querySelector('#accept').addEventListener('click',()=>window.clicks++)").await;
    let answer = act(
        &rig,
        "Later accepted box",
        json!({"action":"check", "checked":true}),
    )
    .await
    .unwrap();
    assert_eq!(answer["result"]["checked"], true);
    assert_eq!(rig.engine.eval("window.clicks").await, 1);
}

#[tokio::test]
async fn radio_prevention_and_unchecking_a_selected_radio_are_unmet() {
    let rig = controls().await;
    for (name, checked) in [("Blocked radio", true), ("Chosen radio", false)] {
        let error = act(&rig, name, json!({"action":"check", "checked":checked}))
            .await
            .expect_err(name);
        assert_eq!(error.outcome, Outcome::Failed);
        assert!(error.text.contains("radio button"), "{error:?}");
    }
}

#[tokio::test]
async fn select_reverts_are_unmet() {
    let rig = controls().await;
    for name in ["Reverted choice", "Later choice"] {
        let error = act(&rig, name, json!({"action":"select", "value":"Beta"}))
            .await
            .expect_err(name);
        assert_eq!(error.outcome, Outcome::Failed);
        assert!(error.text.contains("selected option"), "{error:?}");
    }
}

#[tokio::test]
async fn text_reverts_are_unmet_and_failed_text_is_not_submitted() {
    let rig = controls().await;
    for name in [
        "Reverted text",
        "Later text",
        "Blocked text",
        "Reverted editor",
    ] {
        let error = act(
            &rig,
            name,
            json!({"action":"type", "text":"Wanted text", "submit":true}),
        )
        .await
        .expect_err(name);
        assert_eq!(error.outcome, Outcome::Failed);
        assert!(error.text.contains("requested text"), "{error:?}");
        assert_eq!(rig.engine.eval("window.submissions||0").await, 0);
    }
}

#[tokio::test]
async fn text_reverted_between_verification_and_enter_is_not_submitted() {
    let rig = controls().await;
    *rig.engine.after_verify.lock().unwrap() =
        Some("document.querySelector('#text').value=''".into());
    let error = act(
        &rig,
        "Text",
        json!({"action":"type", "text":"Wanted text", "submit":true}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("requested text"), "{error:?}");
    assert_eq!(rig.engine.eval("window.submissions||0").await, 0);
}

#[tokio::test]
async fn normal_controls_prove_the_requested_state_and_do_not_repeat_input() {
    let rig = controls().await;
    rig.engine.eval("window.clicks=0; document.querySelector('#normal').addEventListener('click',()=>window.clicks++)").await;
    for checked in [true, true, false] {
        let answer = act(
            &rig,
            "Normal box",
            json!({"action":"check", "checked":checked}),
        )
        .await
        .unwrap();
        assert_eq!(answer["result"]["checked"], checked);
        assert_eq!(
            rig.engine
                .eval("document.querySelector('#normal').checked")
                .await,
            checked
        );
    }
    assert_eq!(rig.engine.eval("window.clicks").await, 2);
    act(&rig, "Choice", json!({"action":"select", "value":"Beta"}))
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#select').value")
            .await,
        "b"
    );
    act(
        &rig,
        "Text",
        json!({"action":"type", "text":"A full value", "submit":true}),
    )
    .await
    .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#text').value")
            .await,
        "A full value"
    );
    assert_eq!(rig.engine.eval("window.submissions").await, 1);
    assert_eq!(
        rig.browser
            .sessions()
            .state(SESSION)
            .log
            .last()
            .unwrap()
            .outcome,
        Outcome::Done
    );
    // Keep the public action type exercised alongside the JSON parser.
    assert!(matches!(
        vw_browser::parse(&json!({"action":"check", "ref":"e00000001"})).unwrap(),
        Action::Check { checked: true, .. }
    ));
}

struct Confirm;
impl vw_browser::consent::ConsentConfirmer for Confirm {
    fn confirm(
        &self,
        _: vw_browser::consent::ConsentRequest,
    ) -> vw_browser::consent::ConsentFuture<'_> {
        Box::pin(async { true })
    }
}

#[tokio::test]
async fn native_checkbox_postcondition_is_proved_and_failure_reason_is_preserved() {
    use std::sync::{Arc, atomic::Ordering};
    let rig = controls().await;
    rig.service
        .open(vw_preview::OpenRequest {
            session_id: SESSION.into(),
            url: "https://accounts.google.com/o/oauth2/auth?scope=fixture.wide"
                .parse()
                .unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: true,
            lan: false,
        })
        .await
        .unwrap();
    install_controls(&rig).await;
    rig.browser.set_consent_confirmer(Arc::new(Confirm));
    let answer = act(
        &rig,
        "Normal box",
        json!({"action":"check", "checked":true}),
    )
    .await
    .unwrap();
    assert_eq!(answer["result"]["nativeClick"], true);
    assert_eq!(answer["result"]["checked"], true);
    let error = act(
        &rig,
        "Blocked box",
        json!({"action":"check", "checked":true}),
    )
    .await
    .expect_err("preventDefault after native input must not report done");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(
        error
            .text
            .starts_with("The box is still unchecked; the page did not accept the change."),
        "{error:?}"
    );
    assert!(error.text.contains("Do not repeat the act"), "{error:?}");
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 2);
}
