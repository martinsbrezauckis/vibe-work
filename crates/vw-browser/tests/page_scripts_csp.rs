//! Page JavaScript on a page whose Content-Security-Policy refuses `eval` (`script-src 'self'`, no
//! `'unsafe-eval'`): the expression still runs once in the page's own world, the page's policy stays as it is, and
//! a statement list (which needs `eval`) is refused with the one-expression instruction before anything runs.
//! Pages without such a policy keep the `eval` route (statement lists answer their last value).
//!
//! PREDICT (before the fix): the wrapper ran the expression through an indirect `eval`, which the page's policy
//! refuses, so every expression on such a page failed with "`EvalError`: Refused to evaluate a string as JavaScript
//! because 'unsafe-eval' is not an allowed source of script", while the same expression ran on a page without it.
//! Real Chromium fixture in strict mode (`Rig::strict_eval`): like the app's `WKWebView`/`WebKitGTK` evaluation, the
//! embedder's script itself is not subject to the page's policy, but an `eval` inside it is.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::Actor;
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
/// Gives the fixture page a policy without `'unsafe-eval'` and proves the page's own `eval` is now refused.
async fn strict_page(rig: &Rig) {
    rig.engine.strict_eval().await;
    rig.engine
        .eval("(() => { const m = document.createElement('meta'); m.httpEquiv = 'Content-Security-Policy'; m.content = \"script-src 'self'\"; document.head.append(m); })()")
        .await;
    assert_eq!(
        rig.engine
            .eval("(() => { try { (0, eval)('1'); return 'eval allowed'; } catch (e) { return e.name; } })()")
            .await,
        json!("EvalError"),
        "the fixture's policy must refuse the page's eval"
    );
}
#[tokio::test]
async fn an_expression_runs_once_on_a_page_whose_policy_refuses_eval() {
    let rig = Rig::new().await;
    strict_page(&rig).await;
    let answer = script(
        &rig,
        "(() => { window.cspCount = (window.cspCount || 0) + 1; return {count: window.cspCount, title: document.title}; })()",
    )
    .await
    .unwrap();
    assert_eq!(answer["untrustedPageContent"]["value"]["count"], 1);
    assert_eq!(rig.engine.eval("window.cspCount").await, json!(1));
    let later = script(&rig, "Promise.resolve(21).then(n => n * 2)")
        .await
        .unwrap();
    assert_eq!(later["untrustedPageContent"]["value"], 42);
    assert!(
        script(
            &rig,
            "(() => { throw new Error('strict fixture failure') })()"
        )
        .await
        .unwrap_err()
        .contains("strict fixture failure")
    );
    // The page's own policy is unchanged: its eval is still refused.
    assert_eq!(
        rig.engine
            .eval("(() => { try { (0, eval)('1'); return 'eval allowed'; } catch (e) { return e.name; } })()")
            .await,
        json!("EvalError")
    );
}
#[tokio::test]
async fn a_statement_list_on_a_strict_page_is_refused_before_it_runs() {
    let rig = Rig::new().await;
    strict_page(&rig).await;
    let refused = script(&rig, "window.statementRan = true; 2")
        .await
        .unwrap_err();
    assert!(refused.contains("one expression"), "{refused}");
    assert_eq!(
        rig.engine.eval("window.statementRan === undefined").await,
        json!(true)
    );
}
#[tokio::test]
async fn a_statement_list_still_answers_its_last_value_without_a_policy() {
    let rig = Rig::new().await;
    let answer = script(&rig, "const base = 2; base * 3").await.unwrap();
    assert_eq!(answer["untrustedPageContent"]["value"], 6);
}
