//! Unknown native input must not be replayed by another agent call in the same document.
//! Uses the real browser driver and a fixture that can withhold the post-input receipt.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::atomic::Ordering;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn another_call_cannot_repeat_an_input_with_an_unknown_receipt() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<button id=target style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Again</button>';scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let reference = model
        .refs
        .iter()
        .find(|r| r.name == "Again")
        .unwrap()
        .id
        .clone();
    let action = vw_browser::Action::ClickAt {
        reference,
        x: 50,
        y: 40,
    };
    rig.engine.unreadable_receipt.store(true, Ordering::Relaxed);
    let first = rig
        .browser
        .act(SESSION, action.clone(), Actor::User, &NoNavigation)
        .await;
    let first = first.unwrap_err();
    assert!(first.text.contains("native input was sent"), "{first:?}");
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
    rig.engine
        .unreadable_receipt
        .store(false, Ordering::Relaxed);
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let second = rig
        .browser
        .act(SESSION, action, Actor::Agent, &NoNavigation)
        .await;
    let reason = second.expect_err("unknown input was replayed").text;
    assert!(reason.contains("unresolved outcome"), "{reason}");
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
}
