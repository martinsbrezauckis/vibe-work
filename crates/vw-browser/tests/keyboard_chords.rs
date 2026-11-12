//! Keyboard chords reach real page handlers without accidentally running plain-key defaults.
//! Uses the production browser parser and Chromium fixture; run with the e2e-script feature.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::{Actor, NavFuture, Navigate, ReadOptions};

struct NoNavigation;
impl Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test]
async fn chords_preserve_modifiers_and_do_not_submit_plain_enter() {
    let rig = Rig::new().await;
    rig.engine.eval(r#"(() => {
      document.body.innerHTML = '<form><input aria-label="Editor"><button>Send</button></form>';
      window.keys = []; window.submits = 0;
      document.querySelector('form').onsubmit = e => { e.preventDefault(); window.submits++; };
      document.querySelector('input').onkeydown = e => window.keys.push([e.key,e.code,e.ctrlKey,e.shiftKey,e.altKey,e.metaKey]);
      scrollTo(0,0);
    })()"#).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = &model.refs.iter().find(|r| r.name == "Editor").unwrap().id;
    for key in ["Control+Shift+Z", "Meta+ArrowLeft", "Alt+ArrowDown"] {
        let action =
            vw_browser::parse(&json!({"action":"press", "ref":reference,"key":key})).unwrap();
        rig.browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .unwrap();
    }
    assert_eq!(
        rig.engine.eval("window.keys").await,
        json!([
            ["Z", "KeyZ", true, true, false, false],
            ["ArrowLeft", "ArrowLeft", false, false, false, true],
            ["ArrowDown", "ArrowDown", false, false, true, false]
        ])
    );
    assert_eq!(rig.engine.eval("window.submits").await, 0);
}

#[tokio::test]
async fn shift_tab_moves_back_and_control_a_selects_the_field() {
    let rig = Rig::new().await;
    rig.engine.eval(r#"(() => {
      document.body.innerHTML = '<input id="first" aria-label="First"><input id="last" aria-label="Last" value="hello">';
      document.querySelector('#last').focus(); scrollTo(0,0);
    })()"#).await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    for key in ["Control+a", "Shift+Tab"] {
        let action = vw_browser::parse(&json!({"action":"press", "key":key})).unwrap();
        rig.browser
            .act(SESSION, action, Actor::Agent, &NoNavigation)
            .await
            .unwrap();
    }
    assert_eq!(rig.engine.eval("document.activeElement.id").await, "first");
    assert_eq!(rig.engine.eval("[document.querySelector('#last').selectionStart,document.querySelector('#last').selectionEnd]").await, json!([0,5]));
}

#[test]
fn malformed_chords_are_rejected() {
    for key in [
        "Control+",
        "+Enter",
        "Control+Control+a",
        "Shift+Bogus",
        "Control+\n",
        "Control+Alt+Meta+Shift+Shift+a",
    ] {
        assert!(
            vw_browser::parse(&json!({"action":"press","key":key})).is_err(),
            "{key:?}"
        );
    }
}
