//! Checked input must not borrow a user's trusted click or overlook a second sequence.
//! Uses production isolated scripts and independent Chromium mouse input through the shared engine.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::atomic::Ordering;
use support::{Rig, SESSION};
use vw_browser::{Actor, Outcome, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn prepared() -> (Rig, vw_browser::Action) {
    let rig = Rig::new_in_world(true).await;
    rig.engine
        .eval("new Promise(resolve=>setTimeout(resolve,Math.max(0,350-performance.now())))")
        .await;
    rig.engine.eval(r#"document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">A</button><button id="cover" style="position:fixed;left:-200px;top:20px;width:100px;height:40px">B</button>';
      scrollTo(0,0);window.presses=[];
      window.addEventListener('mousedown',e=>{if(e.isTrusted)window.presses.push(e.target.id)},true)"#).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    (rig, action)
}

async fn overlap(before: bool, move_target: bool) {
    let (rig, action) = prepared().await;
    if move_target {
        rig.engine.eval("document.querySelector('#target').addEventListener('click',e=>{if(e.isTrusted)document.querySelector('#cover').style.left='20px'},{once:true})").await;
    }
    if before {
        rig.engine
            .user_click_before_dispatch
            .store(true, Ordering::Relaxed);
    } else {
        rig.engine
            .user_click_after_dispatch
            .store(true, Ordering::Relaxed);
    }
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    if move_target {
        assert_eq!(
            rig.engine.eval("window.presses").await,
            serde_json::json!(["target"])
        );
        assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
        let error = result.expect_err("moved target was accepted");
        assert_eq!(error.outcome, Outcome::Refused);
        assert!(error.text.contains("the page moved"), "{error:?}");
        return;
    }
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target", if move_target { "cover" } else { "target" }])
    );
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
    let error = result.expect_err("overlapping trusted input was reported as success");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(
        error
            .text
            .contains("the page or the user clicked at the same time, so earlier refs may no longer match; read the page again"),
        "{error:?}"
    );
    let state = rig.browser.sessions().state(SESSION);
    let entry = state.log.last().unwrap();
    assert_eq!(entry.outcome, Outcome::Failed);
    assert!(entry.detail.contains("native input was sent"), "{entry:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn earlier_user_click_cannot_confirm_a_later_click_on_an_overlay() {
    overlap(true, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn earlier_user_click_on_the_same_target_is_ambiguous() {
    overlap(true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn second_sequence_after_dispatch_is_ambiguous() {
    overlap(false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sequence_between_poll_and_collection_is_ambiguous() {
    let (rig, action) = prepared().await;
    *rig.engine.user_click_after_poll.lock().unwrap() = Some((50.0, 40.0));
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target", "target"])
    );
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("clicked at the same time"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn event_created_before_the_mark_cannot_confirm_input_when_delivered_after_it() {
    let (rig, action) = prepared().await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    // Model an OS event queued before the dispatch mark, delivered only afterwards.
    rig.service.evaluate_frames_on_mount(&mount, None,
        "Object.defineProperty(Event.prototype, 'timeStamp', {get:()=>0}); JSON.stringify(true)".into(), None).await.unwrap();
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("clicked at the same time"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_dispatch_mark_refuses_before_input() {
    let (rig, action) = prepared().await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    rig.service.evaluate_frames_on_mount(&mount, None,
        "performance.now = () => {throw new Error('fixture clock unavailable')}; JSON.stringify(true)".into(), None).await.unwrap();
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!([])
    );
    assert_eq!(error.outcome, Outcome::Refused);
}

#[tokio::test(flavor = "multi_thread")]
async fn synthetic_overlap_does_not_contaminate_the_agents_single_sequence() {
    let (rig, action) = prepared().await;
    *rig.engine.after_point_validation.lock().unwrap() = Some(
        "for(const type of ['mousedown','mouseup','click'])document.querySelector('#target').dispatchEvent(new MouseEvent(type,{bubbles:true,clientX:50,clientY:40}))".into());
    rig.browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target"])
    );
}
