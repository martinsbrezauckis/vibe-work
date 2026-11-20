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
// PREDICT: a page script can read protected form values in Full access and return them as JSON.
#[tokio::test]
async fn protected_fields_refuse_page_scripts_before_execution() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<label>Card number <input value=4111111111119917></label><label>PIN <input value=4471></label><label>Password <input type=password value=PWVAL-1></label>'").await;
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(
                &json!({"action":"raw.execute","expression":"window.shouldNotRun = true"}),
            )
            .unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await;
    let error = result.unwrap_err();
    assert!(error.text.contains("Protected field values were withheld"));
    assert_eq!(error.protected_values_withheld, Some(3));
    assert_eq!(
        rig.engine.eval("window.shouldNotRun === undefined").await,
        json!(true)
    );
}
#[tokio::test]
async fn protected_fields_refuse_other_raw_page_outputs() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<label>Card number <input value=4111111111119917></label><label>PIN <input value=4471></label><label>Password <input type=password value=PWVAL-1></label>'").await;
    for request in [
        json!({"action":"page.add_style","css":"body { color: red; }"}),
        json!({"action":"storage.read","area":"local","keys":["fixture"]}),
        // A payment page can copy the typed card number into a cookie.
        json!({"action":"cookies.read"}),
        json!({"action":"network.headers","requestId":"fixture","unredacted":true}),
        json!({"action":"console.read"}),
        json!({"action":"page.errors"}),
    ] {
        let error = rig
            .browser
            .act(
                SESSION,
                vw_browser::parse(&request).unwrap(),
                Actor::Agent,
                &NoNavigation,
            )
            .await
            .unwrap_err();
        assert!(
            error.text.contains("Protected field values were withheld"),
            "{request}: {}",
            error.text
        );
        // The first refusal counts the page's fields and marks the site; later raw actions are refused for the
        // site before the page is counted.
        let first = request["action"] == "page.add_style";
        assert_eq!(
            error.protected_values_withheld,
            Some(if first { 3 } else { 0 }),
            "{request}"
        );
        if !first {
            assert_eq!(
                error.text,
                vw_browser::protected_origins::NOT_RUN_SITE_HAS_FIELD
            );
        }
    }
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
