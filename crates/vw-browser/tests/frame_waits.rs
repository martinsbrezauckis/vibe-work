//! Text waits see the same verified visible documents as browser reads, without retaining a new ref baseline.
//! Uses isolated Chromium frames as the native API fixture; no public consent or input is sent.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::Value;
use std::sync::atomic::Ordering;
use support::{Rig, SESSION};
use vw_browser::{Action, Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn page() -> Rig {
    let rig = Rig::new().await;
    rig.engine.isolate_scripts.store(true, Ordering::Relaxed);
    rig.engine.eval("document.body.innerHTML='<button>Retained main control</button><iframe src=\"https://consent.example.test/privacy\" style=\"position:fixed;inset:0;width:100%;height:100%;border:0\"></iframe>'").await;
    rig.engine
        .wait_for_frame("https://consent.example.test/privacy", "#reject")
        .await;
    rig.engine.eval_in_frame_pages("if(window!==top) document.body.innerHTML='<p>Frame consent ready</p><p hidden>Hidden consent text</p>'").await;
    rig
}

async fn wait(rig: &Rig, text: &str) -> Value {
    rig.browser
        .act(
            SESSION,
            Action::Wait {
                text: Some(text.into()),
                reference: None,
                load: false,
                timeout_ms: 150,
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn text_wait_finds_a_visible_cross_origin_consent_frame() {
    let rig = page().await;
    let receipt = wait(&rig, "Frame consent ready").await;
    assert_eq!(receipt["result"]["met"], true, "{receipt}");
}

#[tokio::test]
async fn text_wait_never_matches_hidden_or_unaddressed_frame_text() {
    let rig = page().await;
    assert_eq!(
        wait(&rig, "Hidden consent text").await["result"]["met"],
        false
    );
    rig.engine
        .eval("document.querySelector('iframe').style.display='none'")
        .await;
    assert_eq!(
        wait(&rig, "Frame consent ready").await["result"]["met"],
        false
    );
    rig.engine
        .eval("document.querySelector('iframe').style.display='block'")
        .await;
    rig.engine
        .unaddressed_children
        .store(true, Ordering::Relaxed);
    assert_eq!(
        wait(&rig, "Frame consent ready").await["result"]["met"],
        false
    );
}

#[tokio::test]
async fn text_wait_can_match_beyond_a_read_text_window() {
    let rig = page().await;
    rig.engine.eval_in_frame_pages("if(window!==top) document.body.innerHTML='<p>'+ 'Long visible context. '.repeat(1500)+'</p><p>Late consent choice</p>'").await;
    rig.browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .unwrap();
    assert_eq!(
        wait(&rig, "Late consent choice").await["result"]["met"],
        true
    );
}
