//! Swapped native targets cannot receive page handlers or default activation.
//! Uses real Chromium input and the production isolated scripts, including early window capture handlers.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn prepared(frame: bool) -> (Rig, String) {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval(r#"
      document.body.innerHTML = '<button id="target" style="position:fixed;left:20px;top:20px;width:100px;height:40px">Press</button><form><button id="cover" style="position:fixed;left:180px;top:20px;width:100px;height:40px">Send</button></form>';
      scrollTo(0,0); window.effects=[];
      for(const type of ['pointerdown','mousedown','mouseup','click','auxclick','dblclick','contextmenu'])
        window.addEventListener(type,e=>{if(e.target.id==='cover')window.effects.push(type)},true);
      document.querySelector('form').onsubmit=e=>{e.preventDefault();window.effects.push('submit')};
    "#).await;
    if frame {
        rig.engine.eval("document.body.insertAdjacentHTML('beforeend','<iframe id=frame style=\"position:fixed;left:180px;top:80px;width:120px;height:100px;border:0\" src=\"https://nested.example.test/child\"></iframe>'); window.addEventListener('message',e=>{if(e.origin==='https://nested.example.test')window.effects.push(e.data)})").await;
        rig.engine
            .wait_for_frame("https://nested.example.test/child", "#nested")
            .await;
        rig.engine.eval_in_frame_pages(r#"if(window!==top){
          document.body.innerHTML='<form><button style="position:fixed;inset:0;width:100%;height:100%">Send</button></form>';
          document.body.dataset.guardEffects='[]';
          const record=type=>{const effects=JSON.parse(document.body.dataset.guardEffects);effects.push(type);document.body.dataset.guardEffects=JSON.stringify(effects);parent.postMessage(type,'*')};
          for(const type of ['pointerdown','mousedown','mouseup','click'])window.addEventListener(type,()=>record(type),true);
          document.querySelector('form').onsubmit=e=>{e.preventDefault();record('submit')};
        }"#).await;
    }
    // The marked-window cases start in mature documents; frames created by their fault hooks stay young.
    rig.engine
        .frame_page_values(
            "new Promise(resolve=>setTimeout(resolve,Math.max(0,350-performance.now())))",
        )
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let id = model
        .refs
        .iter()
        .find(|r| r.name == "Press")
        .unwrap()
        .id
        .clone();
    (rig, id)
}

async fn act(rig: &Rig, id: &str, action: &str) -> vw_browser::ActError {
    rig.browser
        .act_with_sending(
            SESSION,
            vw_browser::parse(&json!({"action":action,"ref":id,"x":50,"y":40})).unwrap(),
            Actor::Agent,
            &NoNavigation,
            false,
        )
        .await
        .expect_err("swapped target was accepted")
}

async fn no_effect(rig: &Rig, error: &vw_browser::ActError) {
    let frames = rig
        .engine
        .frame_page_values("JSON.parse(document.body.dataset.guardEffects || '[]')")
        .await;
    assert!(
        frames
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value == &json!([])),
        "frame effects: {frames}; {error:?}"
    );
    assert_eq!(
        rig.engine.eval("window.effects").await,
        json!([]),
        "{error:?}"
    );
    assert!(error.text.contains("the page moved"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn motion_swap_blocks_early_capture_handlers_and_form_activation_for_every_button() {
    for action in [
        "click",
        "page.double_click",
        "page.right_click",
        "page.middle_click",
    ] {
        let (rig, id) = prepared(false).await;
        rig.engine.move_before_click.store(true, Ordering::Relaxed);
        rig.engine.eval("window.addEventListener('mousemove',()=>document.querySelector('#cover').style.left='20px',{once:true})").await;
        let error = act(&rig, &id, action).await;
        no_effect(&rig, &error).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_coarse_event_timestamp_cannot_bypass_target_enforcement_during_the_mark() {
    let (rig, id) = prepared(false).await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    // Model a native timestamp rounded behind the mark, independently of receipt confirmation.
    rig.service
        .evaluate_frames_on_mount(
            &mount,
            None,
            "Object.defineProperty(Event.prototype, 'timeStamp', {get:()=>0}); true".into(),
            None,
        )
        .await
        .unwrap();
    *rig.engine.after_point_mark.lock().unwrap() =
        Some("document.querySelector('#cover').style.left='20px'".into());
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn normalized_native_event_fields_cannot_bypass_target_enforcement_during_the_mark() {
    for field in ["button", "ctrlKey"] {
        let (rig, id) = prepared(false).await;
        let mount = rig.service.page_mount(SESSION).unwrap();
        // Native compatibility events can normalize fields differently from the requested press.
        let value = if field == "button" { "2" } else { "true" };
        rig.service.evaluate_frames_on_mount(&mount, None,
            format!("Object.defineProperty(MouseEvent.prototype, '{field}', {{get:()=>{value}}}); true"), None).await.unwrap();
        *rig.engine.after_point_mark.lock().unwrap() =
            Some("document.querySelector('#cover').style.left='20px'".into());
        let error = act(&rig, &id, "click").await;
        no_effect(&rig, &error).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interleaved_trusted_sequence_cannot_release_target_enforcement_before_collection() {
    let (rig, id) = prepared(false).await;
    rig.engine.eval("window.coverReady=true").await;
    *rig.engine.after_point_mark.lock().unwrap() =
        Some("document.querySelector('#cover').style.left='20px'".into());
    // Model a trusted sequence delivered ahead of already-admitted agent input. It must not consume the guard.
    rig.engine
        .user_click_after_mark
        .store(true, Ordering::Relaxed);
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_moved_before_dispatch_is_refused_without_native_input() {
    let (rig, id) = prepared(true).await;
    *rig.engine.after_point_validation.lock().unwrap() = Some("document.querySelector('#frame').style.cssText='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0'".into());
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_moved_after_dispatch_mark_has_no_effect_inside_the_frame() {
    let (rig, id) = prepared(true).await;
    *rig.engine.after_point_mark.lock().unwrap() = Some("new Promise(resolve=>{document.querySelector('#frame').style.cssText='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';requestAnimationFrame(()=>requestAnimationFrame(resolve))})".into());
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_the_validated_frame_cannot_retarget_input_to_another_control_at_new_local_coordinates()
 {
    let rig = Rig::new_in_world(true).await;
    rig.engine.eval("document.body.innerHTML='<iframe id=frame style=\"position:fixed;left:20px;top:20px;width:250px;height:100px;border:0\" src=\"https://nested.example.test/child\"></iframe>';scrollTo(0,0);window.effects=[]").await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    rig.engine.eval_in_frame_pages(r#"if(window!==top){
      document.body.innerHTML='<button id="target" style="position:fixed;left:20px;top:20px;width:60px;height:40px">Press</button><form><button id="cover" style="position:fixed;left:120px;top:20px;width:60px;height:40px">Send</button></form>';
      document.body.dataset.guardEffects='[]';
      const record=type=>{const effects=JSON.parse(document.body.dataset.guardEffects);effects.push(type);document.body.dataset.guardEffects=JSON.stringify(effects)};
      for(const type of ['pointerdown','mousedown','mouseup','click'])window.addEventListener(type,e=>{if(e.target.id==='cover')record(type)},true);
      document.querySelector('form').onsubmit=e=>{e.preventDefault();record('submit')};
    }"#).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let id = &model
        .refs
        .iter()
        .find(|item| item.name == "Press")
        .unwrap()
        .id;
    *rig.engine.after_point_mark.lock().unwrap() = Some("new Promise(resolve=>{document.querySelector('#frame').style.left='-100px';requestAnimationFrame(()=>requestAnimationFrame(resolve))})".into());
    let error = act(&rig, id, "click").await;
    no_effect(&rig, &error).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn person_clicks_keep_handlers_and_default_form_activation_after_a_blocked_agent_press() {
    let (rig, id) = prepared(false).await;
    *rig.engine.after_point_mark.lock().unwrap() =
        Some("document.querySelector('#cover').style.left='20px'".into());
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
    rig.engine.person_click(50.0, 40.0).await;
    let effects = rig.engine.eval("window.effects").await;
    assert!(
        effects.as_array().unwrap().contains(&Value::from("click")),
        "{effects}"
    );
    assert!(
        effects.as_array().unwrap().contains(&Value::from("submit")),
        "{effects}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_click_on_the_swapped_target_during_the_mark_is_blocked_and_refuses_the_agent() {
    let (rig, id) = prepared(false).await;
    *rig.engine.after_point_mark.lock().unwrap() =
        Some("document.querySelector('#cover').style.left='20px'".into());
    rig.engine
        .person_click_after_mark
        .store(true, Ordering::Relaxed);
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
    assert_eq!(error.outcome, vw_browser::Outcome::Refused);
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
    let state = rig.browser.sessions().state(SESSION);
    let receipt = state.log.last().unwrap();
    assert_eq!(receipt.outcome, vw_browser::Outcome::Refused);
    assert!(receipt.detail.contains("the page moved"), "{receipt:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_frame_created_after_the_mark_cannot_receive_the_native_press() {
    let (rig, id) = prepared(false).await;
    rig.engine
        .eval("window.addEventListener('message',e=>window.effects.push(e.data))")
        .await;
    *rig.engine.after_point_mark.lock().unwrap() = Some(r#"new Promise(resolve=>{
      const frame=document.createElement('iframe');
      frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
      frame.srcdoc='<button id="new" style="position:fixed;inset:0;width:100%;height:100%" onclick="this.dataset.clicked=1;parent.postMessage(\'click\',\'*\')">Send</button>';
      frame.onload=()=>requestAnimationFrame(()=>requestAnimationFrame(resolve));
      document.body.append(frame);
    })"#.into());
    let error = act(&rig, &id, "click").await;
    assert_eq!(rig.engine.eval("document.querySelector('iframe').contentDocument.querySelector('#new').dataset.clicked || null").await, Value::Null, "{error:?}");
    assert!(error.text.contains("the page moved"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_initial_blank_frame_created_after_the_mark_is_guarded_before_parent_installed_handlers()
{
    let (rig, id) = prepared(false).await;
    *rig.engine.after_point_mark.lock().unwrap() = Some(r#"new Promise(resolve=>{
      const frame=document.createElement('iframe');
      frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
      document.body.append(frame);
      frame.contentDocument.body.innerHTML='<button id="new" style="position:fixed;inset:0;width:100%;height:100%">Send</button>';
      frame.contentWindow.addEventListener('click',()=>frame.contentDocument.body.dataset.clicked='1',true);
      requestAnimationFrame(()=>requestAnimationFrame(resolve));
    })"#.into());
    let error = act(&rig, &id, "click").await;
    assert_eq!(
        rig.engine
            .eval("document.querySelector('iframe').contentDocument.body.dataset.clicked || null")
            .await,
        Value::Null,
        "{error:?}"
    );
    assert!(error.text.contains("the page moved"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cross_origin_document_created_after_the_mark_blocks_handlers_and_default_activation() {
    let (rig, id) = prepared(false).await;
    *rig.engine.after_point_mark.lock().unwrap() = Some(
        r"new Promise(resolve=>{
      const frame=document.createElement('iframe');
      frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
      frame.src='https://nested.example.test/new-guard';
      frame.onload=()=>requestAnimationFrame(()=>requestAnimationFrame(resolve));
      document.body.append(frame);
    })"
        .into(),
    );
    let error = act(&rig, &id, "click").await;
    no_effect(&rig, &error).await;
    assert_eq!(
        rig.engine
            .frame_page_values("document.body.dataset.guardEffects || null")
            .await,
        json!([null, "[]"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_submission_cannot_outlive_the_new_document_grace_period() {
    let (rig, id) = prepared(false).await;
    *rig.engine.after_point_mark.lock().unwrap() = Some(r#"new Promise(resolve=>{
      const frame=document.createElement('iframe');
      frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
      frame.srcdoc='<button id="new" style="position:fixed;inset:0;width:100%;height:100%" onclick="this.dataset.clicked=1">Send</button>';
      frame.onload=()=>setTimeout(resolve,400);
      document.body.append(frame);
    })"#.into());
    let error = act(&rig, &id, "click").await;
    assert_eq!(rig.engine.eval("document.querySelector('iframe').contentDocument.querySelector('#new').dataset.clicked || null").await, Value::Null, "{error:?}");
    assert_eq!(rig.engine.native_clicks.load(Ordering::Relaxed), 0);
    assert_eq!(error.outcome, vw_browser::Outcome::Refused);
    assert!(error.text.contains("the page moved"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_native_sequences_keep_their_creation_time_and_fresh_person_input_is_released() {
    for (button, count) in [("left", 1), ("left", 2), ("right", 1), ("middle", 1)] {
        let (rig, _) = prepared(false).await;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        rig.engine.eval(r#"new Promise(resolve=>{
          const frame=document.createElement('iframe');
          frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
          frame.srcdoc='<form><button style="position:fixed;inset:0;width:100%;height:100%">Send</button></form><script>document.body.dataset.effects="[]";const record=type=>{const effects=JSON.parse(document.body.dataset.effects);effects.push(type);document.body.dataset.effects=JSON.stringify(effects)};for(const type of ["pointerdown","mousedown","mouseup","click","auxclick","dblclick","contextmenu"])window.addEventListener(type,()=>record(type),true);document.querySelector("form").onsubmit=e=>{e.preventDefault();record("submit")};</script>';
          frame.onload=()=>setTimeout(resolve,400);
          document.body.append(frame);
        })"#).await;
        rig.engine.timestamped_click(timestamp, button, count).await;
        assert_eq!(rig.engine.eval("JSON.parse(document.querySelector('iframe').contentDocument.body.dataset.effects)").await, json!([]));
        rig.engine.person_click(50.0, 40.0).await;
        let effects = rig
            .engine
            .eval(
                "JSON.parse(document.querySelector('iframe').contentDocument.body.dataset.effects)",
            )
            .await;
        assert!(
            effects.as_array().unwrap().contains(&json!("submit")),
            "{effects}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn new_documents_release_person_input_without_receipt_or_host_cleanup() {
    let (rig, _) = prepared(false).await;
    rig.engine.eval(r#"new Promise(resolve=>{
      const frame=document.createElement('iframe');
      frame.style='position:fixed;left:20px;top:20px;width:120px;height:100px;border:0';
      frame.srcdoc='<form><button id="new" style="position:fixed;inset:0;width:100%;height:100%">Send</button></form><script>document.querySelector("form").onsubmit=e=>{e.preventDefault();document.body.dataset.submitted="1"}</script>';
      frame.onload=()=>requestAnimationFrame(()=>requestAnimationFrame(resolve));
      document.body.append(frame);
    })"#).await;
    rig.engine.person_click(50.0, 40.0).await;
    assert_eq!(
        rig.engine
            .eval("document.querySelector('iframe').contentDocument.body.dataset.submitted || null")
            .await,
        Value::Null
    );
    rig.engine
        .frame_page_values(
            "new Promise(resolve=>setTimeout(resolve,Math.max(0,350-performance.now())))",
        )
        .await;
    rig.engine.person_click(50.0, 40.0).await;
    assert_eq!(
        rig.engine
            .eval("document.querySelector('iframe').contentDocument.body.dataset.submitted")
            .await,
        json!("1")
    );
}
