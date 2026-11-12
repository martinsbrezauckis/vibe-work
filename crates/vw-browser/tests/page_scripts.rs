//! Main-world page scripts finish once, report failures, and cannot cross the vault boundary.
//! Real Chromium fixture checks that ordinary agent reads remain usable after page JavaScript.
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
async fn script(rig: &Rig, expression: &str) -> Result<Value, String> {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"raw.execute","expression":expression}))?,
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|e| e.text)
}
#[tokio::test]
async fn page_script_resolves_once_and_reports_real_failures() {
    let rig = Rig::new_in_world(true).await;
    let answer=script(&rig,"Promise.resolve().then(() => { window.fixtureCount = (window.fixtureCount || 0) + 1; return {count: window.fixtureCount, title: document.title}; })").await.unwrap();
    assert_eq!(answer["untrustedPageContent"]["value"]["count"], 1);
    assert_eq!(rig.engine.eval("window.fixtureCount").await, json!(1));
    assert!(
        rig.service
            .admit_vault_fill(SESSION)
            .unwrap_err()
            .contains("JavaScript")
    );
    assert!(
        script(&rig, "(() => { throw new Error('fixture failure') })()")
            .await
            .unwrap_err()
            .contains("fixture failure")
    );
    assert!(
        rig.browser
            .read(SESSION, ReadOptions::default(), false, Actor::Agent)
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn vault_filled_document_refuses_script_before_evaluation() {
    let rig = Rig::new().await;
    rig.service.block_shots_on_this_document(SESSION);
    assert!(
        script(&rig, "window.shouldNotRun = true")
            .await
            .unwrap_err()
            .contains("vault")
    );
    assert_eq!(
        rig.engine.eval("window.shouldNotRun === undefined").await,
        json!(true)
    );
}

#[tokio::test]
async fn inserted_styles_are_measured_and_pending_scripts_time_out_without_replay() {
    let rig = Rig::new().await;
    let answer = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(
                &json!({"action":"page.add_style","css":"body { color: rgb(1, 2, 3); }"}),
            )
            .unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(answer["untrustedPageContent"]["value"]["applied"], true);
    assert_eq!(
        rig.engine
            .eval("getComputedStyle(document.body).color")
            .await,
        json!("rgb(1, 2, 3)")
    );
    let pending=vw_browser::parse(&json!({"action":"raw.execute","expression":"new Promise(() => {window.pendingCount = (window.pendingCount || 0) + 1;})","timeoutMs":100})).unwrap();
    assert!(
        rig.browser
            .act(SESSION, pending, Actor::Agent, &NoNavigation)
            .await
            .unwrap_err()
            .text
            .contains("deadline")
    );
    assert_eq!(rig.engine.eval("window.pendingCount").await, json!(1));
}
