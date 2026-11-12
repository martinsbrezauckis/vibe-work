//! Frame creation, removal and navigation invalidate a checked click's isolated receipt.
//! Uses real Chromium frame lifecycle and trusted mouse input through the production browser service.
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

async fn prepared(child: bool) -> (Rig, vw_browser::Action) {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval(r#"document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">A</button>';
      scrollTo(0,0);window.presses=[];
      window.addEventListener('mousedown',e=>{if(e.isTrusted)window.presses.push(e.target.id)},true)"#).await;
    if child {
        rig.engine.eval("new Promise(resolve=>{const f=document.createElement('iframe');f.id='child';f.style='position:fixed;left:200px;top:100px';f.onload=resolve;f.srcdoc='<p>Armed child</p>';document.body.append(f)})").await;
    }
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

async fn changed(rig: &Rig, action: vw_browser::Action) {
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
    let error = result.expect_err("changed frame registry was reported as success");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(
        error.text.contains(
            "the page changed while clicking, so earlier refs no longer match; read the page again"
        ),
        "{error:?}"
    );
    let state = rig.browser.sessions().state(SESSION);
    let entry = state.log.last().unwrap();
    assert_eq!(entry.outcome, Outcome::Failed);
    assert!(entry.detail.contains("native input was sent"), "{entry:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn user_click_after_mark_cannot_confirm_agent_input_in_a_new_frame() {
    let (rig, action) = prepared(false).await;
    rig.engine.eval(r#"window.coverReady=false;
      document.querySelector('#target').addEventListener('click',e=>{
        if(!e.isTrusted)return;
        const f=document.createElement('iframe');f.style='position:fixed;left:20px;top:20px;width:100px;height:40px;border:0;z-index:100';
        f.onload=()=>{f.contentWindow.addEventListener('mousedown',e=>{if(e.isTrusted)window.presses.push(e.target.id)},true);window.coverReady=true};
        f.srcdoc='<button id="cover" style="position:fixed;inset:0;width:100%;height:100%">B</button>';document.body.append(f);
      },{once:true})"#).await;
    rig.engine
        .user_click_after_mark
        .store(true, Ordering::Relaxed);
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.eval("window.presses").await,
        serde_json::json!(["target"])
    );
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
    let error = result.expect_err("user receipt confirmed input delivered to an unarmed frame");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(
        error.text.contains(
            "the page changed while clicking, so earlier refs no longer match; read the page again"
        ),
        "{error:?}"
    );
    assert!(error.text.contains("native input was sent"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_armed_frame_between_poll_and_collection_fails_closed() {
    let (rig, action) = prepared(true).await;
    *rig.engine.after_receipt_poll.lock().unwrap() =
        Some("document.querySelector('#child').remove()".into());
    changed(&rig, action).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigated_armed_frame_between_poll_and_collection_fails_closed() {
    let (rig, action) = prepared(true).await;
    *rig.engine.after_receipt_poll.lock().unwrap() = Some("new Promise(resolve=>{const f=document.querySelector('#child');f.onload=resolve;f.srcdoc='<p>New document at the same URL</p>'})".into());
    changed(&rig, action).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_frame_between_poll_and_collection_fails_closed() {
    let (rig, action) = prepared(false).await;
    *rig.engine.after_receipt_poll.lock().unwrap() = Some("new Promise(resolve=>{const f=document.createElement('iframe');f.onload=()=>{f.remove();resolve()};f.srcdoc='<p>Transient frame</p>';document.body.append(f)})".into());
    changed(&rig, action).await;
}

/// A frame whose script cannot run cannot report the events of a checked click: such a click is refused before any
/// input, while the page is still read (the frame left out and named).
#[tokio::test(flavor = "multi_thread")]
async fn a_frame_that_cannot_be_read_refuses_a_checked_click_before_input() {
    let (rig, action) = prepared(true).await;
    rig.engine
        .unreadable_children
        .store(true, Ordering::Relaxed);
    let (answer, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), false, Actor::Agent)
        .await
        .expect("an unreadable frame failed the read");
    assert_eq!(
        answer["untrustedPageContent"]["unreadFrames"],
        serde_json::json!(["a frame could not be read"]),
        "{answer}"
    );
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = match action {
        vw_browser::Action::ClickAt { x, y, .. } => vw_browser::Action::ClickAt {
            reference: target.id.clone(),
            x,
            y,
        },
        other => other,
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?}"
    );
    let error = result.expect_err("a checked click went ahead with an uncountable frame");
    assert!(error.text.contains("Not run"), "{error:?}");
}

/// The same refusal when only the read finds the frame unreadable and the click's own frame registry would answer
/// for it (`WebKit` on macOS): the read's verdict decides before any input.
#[tokio::test(flavor = "multi_thread")]
async fn a_frame_only_the_read_cannot_observe_refuses_a_checked_click_before_input() {
    let (rig, _) = prepared(true).await;
    rig.engine
        .unreadable_children_in_reads
        .store(true, Ordering::Relaxed);
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .expect("an unreadable frame failed the read");
    assert_eq!(model.unread_frames, ["a frame could not be read"]);
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?}"
    );
    let error =
        result.expect_err("a checked click went ahead with a frame the read could not observe");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(
        error
            .text
            .contains("Not run: a frame on this page cannot report"),
        "{error:?}"
    );
}

/// A child that becomes unreadable after the read, while the main document stays loaded, refuses a direct
/// coordinate click before any input: the click observes the frames again instead of trusting the earlier read.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_that_becomes_unreadable_after_the_read_refuses_a_coordinate_click_before_input() {
    let (rig, action) = prepared(true).await;
    rig.engine
        .unreadable_children_in_reads
        .store(true, Ordering::Relaxed);
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?}"
    );
    let error =
        result.expect_err("a coordinate click went ahead with a frame that became unreadable");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(
        error
            .text
            .contains("Not run: a frame on this page cannot report"),
        "{error:?}"
    );
}

/// A scriptless sandbox inside a shadow root of `mode`, with no frame answer for it (the Linux frame registry never
/// sees a frame sandboxed without scripts from its creation), next to a clickable main-page target; returns the
/// read's unreadable reasons and the coordinate click's result.
async fn silent_frame_in_a_shadow_root(
    mode: &str,
) -> (
    Rig,
    Vec<String>,
    Result<serde_json::Value, vw_browser::ActError>,
) {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval(&format!(r#"new Promise(resolve=>{{
      document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">A</button><div id="host"></div>';
      scrollTo(0,0);
      const root=document.querySelector('#host').attachShadow({{mode:'{mode}'}});
      const f=document.createElement('iframe');f.sandbox='';f.style='position:fixed;left:200px;top:100px;width:200px;height:60px';
      f.srcdoc='<button>Hidden sandbox</button>';f.onload=()=>setTimeout(resolve,200);root.append(f)}})"#)).await;
    rig.engine
        .silent_inline_children
        .store(true, Ordering::Relaxed);
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .expect("a silent frame failed the read");
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    (rig, model.unread_frames, result)
}

/// A frame inside a shadow root is no child window of the page (`window.length` counts only the document tree) and
/// no element the parent's frame listing finds; when no frame answer arrives for it either, the read still names
/// it as unreadable and a checked click is refused before any input.
#[tokio::test(flavor = "multi_thread")]
async fn a_silent_frame_in_a_shadow_root_refuses_a_checked_click_before_input() {
    let (rig, unread, result) = silent_frame_in_a_shadow_root("open").await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?}"
    );
    let error = result.expect_err("a checked click went ahead with a frame nothing could observe");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(
        error
            .text
            .contains("Not run: a frame on this page cannot report"),
        "{error:?}"
    );
    assert_eq!(unread, ["a frame could not be read"]);
}

/// A silent frame in an open shadow root is named unreadable whatever reasons other frames gave: here a nested frame
/// chain whose parent and child both have no native address gives two reasons of its own, as many as the frames it
/// and the silent one leave out. The read still names the page's frames unreadable and a checked click is refused
/// before any input. (The outer frame's page embeds a frame of its own.)
#[tokio::test(flavor = "multi_thread")]
async fn other_frames_reasons_never_hide_a_silent_frame_in_a_shadow_root() {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval(r#"new Promise(resolve=>{
      document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">A</button><div id="host"></div>';
      scrollTo(0,0);
      const root=document.querySelector('#host').attachShadow({mode:'open'});
      const f=document.createElement('iframe');f.sandbox='';f.style='position:fixed;left:200px;top:100px;width:200px;height:60px';
      f.srcdoc='<button>Hidden sandbox</button>';root.append(f);
      const outer=document.createElement('iframe');outer.style='position:fixed;left:200px;top:200px;width:200px;height:100px';
      outer.src='https://frame.example.test/';outer.onload=()=>setTimeout(resolve,200);document.body.append(outer)})"#).await;
    rig.engine
        .silent_inline_children
        .store(true, Ordering::Relaxed);
    rig.engine
        .unaddressed_children
        .store(true, Ordering::Relaxed);
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .expect("frames without answers or addresses failed the read");
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?} (unread {:?})",
        model.unread_frames
    );
    let error = result.expect_err("a checked click went ahead with a frame nothing could observe");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(
        error
            .text
            .contains("Not run: a frame on this page cannot report"),
        "{error:?}"
    );
    assert!(
        model
            .unread_frames
            .iter()
            .any(|r| r == "a frame could not be read"),
        "{:?}",
        model.unread_frames
    );
}

/// A removed frame's handle that still answers from its detached document is no frame of the page: the read names no
/// unreadable frame for it. While it stays in the registry a checked click is refused before any input (its later
/// removal would change the registry during the click).
#[tokio::test(flavor = "multi_thread")]
async fn a_lingering_removed_frame_handle_refuses_a_checked_click_before_input() {
    let (rig, _) = prepared(true).await;
    rig.engine
        .detached_child_evaluations
        .store(u64::MAX, Ordering::Relaxed);
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .expect("a detached frame answer failed the read");
    assert!(model.unread_frames.is_empty(), "{:?}", model.unread_frames);
    let target = model.refs.iter().find(|r| r.name == "A").unwrap();
    let action = vw_browser::Action::ClickAt {
        reference: target.id.clone(),
        x: 50,
        y: 40,
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        0,
        "native input was sent: {result:?}"
    );
    let error =
        result.expect_err("a checked click went ahead while a removed frame's handle lingered");
    assert_eq!(error.outcome, Outcome::Refused, "{error:?}");
    assert!(
        error
            .text
            .contains("Not run: the page is still changing; read the page again"),
        "{error:?}"
    );
}

/// A removed frame's handle that leaves the registry within the wait does not stop the checked click.
#[tokio::test(flavor = "multi_thread")]
async fn a_removed_frame_handle_that_goes_within_the_wait_lets_the_checked_click_proceed() {
    let (rig, action) = prepared(true).await;
    rig.engine
        .detached_child_evaluations
        .store(3, Ordering::Relaxed);
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine
            .detached_child_evaluations
            .load(Ordering::Relaxed),
        0,
        "the removed frame's handle was not seen through the click: {result:?}"
    );
    assert_eq!(
        rig.engine.native_clicks.load(Ordering::Relaxed),
        1,
        "{result:?}"
    );
    result.expect("a checked click was refused after the removed frame's handle had gone");
}
