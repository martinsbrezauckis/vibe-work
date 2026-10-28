//! Navigation receipts with real page clicks and delayed engine lifecycle callbacks.
//! Uses Chromium support; verifies completion, bounded pending loads and the window's refusal.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::time::{Duration, Instant};
use support::{Rig, SESSION};
use vw_browser::{Action, Actor, NavFuture, Navigate, Outcome, ReadOptions};
use vw_preview::PageEvent;

struct NoNavigation;
impl Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> NavFuture<'a> {
        Box::pin(async { Err("unexpected explicit navigation".into()) })
    }
}

async fn fixture(outcome: &str) -> (Rig, Action) {
    let rig = Rig::new().await;
    rig.engine.eval("document.head.innerHTML='<title>Before</title>'; document.body.innerHTML='<button onclick=\"window.clicked=true\">Continue</button>'; scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let action = Action::Click {
        reference: model.refs[0].id.clone(),
    };
    let mount = rig.service.page_mount(SESSION).unwrap();
    let engine = rig.engine.clone();
    let hooks = engine.hooks();
    let outcome = outcome.to_owned();
    *rig.engine.on_click.lock().unwrap() = Some(Box::new(move || {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            if outcome == "refused" {
                assert!(!hooks.allow_navigation(&mount.label, &"file:///refused".parse().unwrap()));
                return;
            }
            let url = "https://93.184.215.14/next".parse().unwrap();
            hooks.page_event(&mount.label, PageEvent::Started(url));
            if outcome.starts_with("loaded") {
                tokio::time::sleep(Duration::from_millis(150)).await;
                engine.eval("history.pushState({}, '', '/next'); document.title='Destination'; document.body.innerHTML='<h1>Arrived</h1>'").await;
                hooks.page_event(
                    &mount.label,
                    PageEvent::Finished("https://93.184.215.14/next".parse().unwrap()),
                );
                if outcome == "loaded-with-frame" {
                    assert!(
                        !hooks.allow_navigation(
                            &mount.label,
                            &"file:///refused-frame".parse().unwrap(),
                        )
                    );
                }
            }
        });
    }));
    (rig, action)
}

#[tokio::test]
async fn click_waits_for_delayed_navigation_and_reports_the_new_page() {
    let (rig, action) = fixture("loaded").await;
    let receipt = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(receipt["untrustedPageContent"]["title"], "Destination");
    assert_eq!(receipt["loading"], false);
}

#[tokio::test]
async fn loaded_click_keeps_a_blocked_frame_in_diagnostics_without_failing_the_page() {
    let (rig, action) = fixture("loaded-with-frame").await;
    let receipt = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(receipt["untrustedPageContent"]["title"], "Destination");
    assert_eq!(receipt["loading"], false);
    assert_eq!(
        receipt["untrustedPageContent"]["refusedNavigations"][0]["url"],
        "file:///refused-frame",
    );
}

#[tokio::test]
async fn click_reports_still_loading_within_twelve_seconds() {
    let (rig, action) = fixture("pending").await;
    let start = Instant::now();
    let receipt = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(12));
    assert_eq!(receipt["loading"], true);
    assert!(
        receipt["Outcome"]
            .as_str()
            .unwrap()
            .contains("still loading")
    );
}

#[tokio::test]
async fn click_reports_the_navigation_refusal_instead_of_success() {
    let (rig, action) = fixture("refused").await;
    let error = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await
        .unwrap_err();
    assert_eq!(error.outcome, Outcome::Refused);
    assert!(
        error.text.starts_with("Not opened: file:///refused"),
        "{error:?}"
    );
}

#[tokio::test]
async fn another_refusal_of_the_same_url_survives_the_receipt_mark() {
    let rig = Rig::new().await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    let url = "file:///refused".parse().unwrap();
    assert!(!rig.engine.hooks().allow_navigation(&mount.label, &url));
    let mark = rig.service.refusal_mark(SESSION);
    assert!(!rig.engine.hooks().allow_navigation(&mount.label, &url));
    assert_eq!(rig.service.refusals_since(SESSION, mark).len(), 1);
}
