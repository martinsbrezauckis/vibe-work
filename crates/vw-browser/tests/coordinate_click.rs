//! Coordinate clicks use trusted input and retain the observed node identity.
//! Drives real page scripts through the shared engine; native dispatch records independently of DOM effects.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn button() -> (Rig, String) {
    button_in_world(false).await
}
async fn button_in_world(isolated: bool) -> (Rig, String) {
    let rig = Rig::new().await;
    rig.engine
        .isolate_scripts
        .store(isolated, std::sync::atomic::Ordering::Relaxed);
    rig.engine.eval("document.body.innerHTML = '<button id=target style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Press</button>'; scrollTo(0,0)").await;
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
#[tokio::test(flavor = "multi_thread")]
async fn point_click_uses_native_input() {
    let (rig, id) = button().await;
    let action =
        vw_browser::parse(&serde_json::json!({"action":"click","ref":id,"x":50,"y":40})).unwrap();
    rig.browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn identical_replacement_is_refused_at_the_point() {
    let (rig, id) = button().await;
    rig.engine
        .eval("const b = document.querySelector('#target'); b.replaceWith(b.cloneNode(true))")
        .await;
    let action =
        vw_browser::parse(&serde_json::json!({"action":"click","ref":id,"x":50,"y":40})).unwrap();
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert!(result.is_err(), "replacement node was clicked: {result:?}");
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn covering_overlay_refuses_and_raw_point_binds_the_observed_node() {
    let (rig, id) = button().await;
    rig.engine.eval("document.body.insertAdjacentHTML('beforeend','<div id=cover style=\"position:fixed;inset:0;background:white\"></div>')").await;
    let action =
        vw_browser::parse(&serde_json::json!({"action":"click","ref":id,"x":50,"y":40})).unwrap();
    assert!(
        rig.browser
            .act(SESSION, action, Actor::User, &NoNavigation)
            .await
            .is_err()
    );
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    rig.engine
        .eval("document.querySelector('#cover').remove()")
        .await;
    let action = vw_browser::parse(&serde_json::json!({"action":"click","x":50,"y":40})).unwrap();
    rig.browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn moved_or_detached_frame_refuses_native_dispatch() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<iframe style=\"position:fixed;left:0;top:0;width:250px;height:150px\" src=\"https://nested.example.test/child\"></iframe>';scrollTo(0,0)").await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let item = model
        .refs
        .iter()
        .find(|r| r.name == "Nested button")
        .unwrap();
    let b = item.bounds.unwrap();
    let action = vw_browser::parse(
        &serde_json::json!({"action":"click","ref":item.id,"x":b.x+b.width/2,"y":b.y+b.height/2}),
    )
    .unwrap();
    rig.engine
        .eval("document.querySelector('iframe').style.left='30px'")
        .await;
    assert!(
        rig.browser
            .act(SESSION, action.clone(), Actor::User, &NoNavigation)
            .await
            .is_err()
    );
    rig.engine
        .eval("document.querySelector('iframe').remove()")
        .await;
    assert!(
        rig.browser
            .act(SESSION, action, Actor::User, &NoNavigation)
            .await
            .is_err()
    );
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_container_identity_cannot_authorize_a_click_inside_its_document() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<iframe style=\"position:fixed;left:0;top:0;width:250px;height:150px\" src=\"https://nested.example.test/child\"></iframe>';scrollTo(0,0)").await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let frame = model.frames.first().unwrap();
    let action =
        vw_browser::parse(&serde_json::json!({"action":"click","ref":frame.id,"x":40,"y":20}))
            .unwrap();
    assert!(
        rig.browser
            .act(SESSION, action, Actor::User, &NoNavigation)
            .await
            .is_err()
    );
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn authorized_native_ref_click_keeps_automatic_scroll() {
    struct Confirm;
    impl vw_browser::consent::ConsentConfirmer for Confirm {
        fn confirm(
            &self,
            _: vw_browser::consent::ConsentRequest,
        ) -> vw_browser::consent::ConsentFuture<'_> {
            Box::pin(async { true })
        }
    }
    let rig = Rig::new().await;
    rig.service
        .open(vw_preview::OpenRequest {
            session_id: SESSION.into(),
            url: "https://accounts.google.com/o/oauth2/auth?scope=fixture.wide"
                .parse()
                .unwrap(),
            allowed_ports: vec![],
            downloads_dir: None,
            wait: true,
            lan: false,
        })
        .await
        .unwrap();
    rig.browser
        .set_consent_confirmer(std::sync::Arc::new(Confirm));
    rig.engine.eval("document.body.innerHTML='<button id=target style=\"position:absolute;left:20px;top:700px;width:100px;height:40px\" onclick=\"this.dataset.clicked=1\">Below</button>';scrollTo(0,0)").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let id = model
        .refs
        .iter()
        .find(|r| r.name == "Below")
        .unwrap()
        .id
        .clone();
    rig.browser
        .act(
            SESSION,
            vw_browser::Action::Click { reference: id },
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap();
    assert_eq!(
        rig.engine
            .eval("document.querySelector('#target').dataset.clicked")
            .await,
        serde_json::json!("1")
    );
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_world_cannot_forge_checked_click_admission() {
    let (rig, id) = button().await;
    rig.engine
        .page_world
        .store(true, std::sync::atomic::Ordering::Relaxed);
    rig.engine.eval("document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;inset:0\">Cover</button>'); const original = JSON.stringify; JSON.stringify = function(value, ...args) { return original(value && typeof value === 'object' && 'ok' in value ? {...value,ok:true} : value, ...args); }").await;
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await;
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0,
        "page-world forgery sent native input: {result:?}"
    );
    assert!(
        result
            .unwrap_err()
            .text
            .contains("require isolated frame validation")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn overlay_after_validation_is_reported_as_misdirected_input() {
    overlay_is_reported(None).await;
}

/// The page's receipt reaches the browser late (a slow evaluation, as under load): the diagnosis already observed
/// is still reported, and the input is not repeated.
#[tokio::test(flavor = "multi_thread")]
async fn overlay_is_reported_even_when_the_receipt_arrives_late() {
    overlay_is_reported(Some("new Promise(resolve => setTimeout(resolve, 1200))")).await;
}

async fn overlay_is_reported(late_receipt: Option<&str>) {
    let (rig, id) = button_in_world(true).await;
    *rig.engine.after_receipt_poll.lock().unwrap() = late_receipt.map(str::to_owned);
    *rig.engine.after_point_validation.lock().unwrap() = Some("document.body.insertAdjacentHTML('beforeend','<button id=cover style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Cover</button>')".into());
    let result = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await;
    // 0 here means a check before the input refused it; the answer says which.
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "{result:?}"
    );
    let error = result.expect_err("moved page was reported as an intended click");
    assert!(
        error
            .text
            .contains("the page moved: the click landed on button Cover"),
        "{error:?}"
    );
    assert_eq!(error.outcome, vw_browser::Outcome::Failed);
    let state = rig.browser.sessions().state(SESSION);
    let receipt = state.log.last().unwrap();
    assert_eq!(receipt.outcome, vw_browser::Outcome::Failed);
    assert!(receipt.detail.contains("the page moved"), "{receipt:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_receipt_ignores_page_globals_and_synthetic_events() {
    let (rig, id) = button_in_world(true).await;
    *rig.engine.after_point_validation.lock().unwrap() = Some(r#"
      JSON.stringify = () => '{"ok":true,"matches":true}';
      for (const type of ['mousedown','mouseup','click']) document.querySelector('#target').dispatchEvent(new MouseEvent(type,{bubbles:true}));
      document.body.insertAdjacentHTML('beforeend','<button style="position:fixed;left:20px;top:20px;width:100px;height:40px">Cover</button>');
    "#.into());
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .text
            .contains("the page moved: the click landed on button Cover"),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_event_receipt_never_reports_success() {
    let (rig, id) = button_in_world(true).await;
    rig.engine
        .eval("window.addEventListener('click', e => e.stopImmediatePropagation(), true)")
        .await;
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(
        error
            .text
            .contains("event-time target could not be confirmed"),
        "{error:?}"
    );
    assert_eq!(error.outcome, vw_browser::Outcome::Failed);
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_receipt_accepts_a_descendant_and_does_not_reuse_an_old_event() {
    let (rig, id) = button_in_world(true).await;
    rig.engine
        .eval("document.querySelector('#target').innerHTML='<span>Press</span>'")
        .await;
    let action = vw_browser::Action::ClickAt {
        reference: id,
        x: 50,
        y: 40,
    };
    rig.browser
        .act(SESSION, action.clone(), Actor::User, &NoNavigation)
        .await
        .unwrap();
    *rig.engine.after_point_validation.lock().unwrap() = Some("document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Cover</button>')".into());
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert!(error.text.contains("the page moved"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn covered_child_after_validation_records_the_actual_parent_frame() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<iframe style=\"position:fixed;left:0;top:0;width:250px;height:150px\" src=\"https://nested.example.test/child\"></iframe>';scrollTo(0,0)").await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let item = model
        .refs
        .iter()
        .find(|r| r.name == "Nested button")
        .unwrap();
    let b = item.bounds.unwrap();
    // Input to a cross-site child is routed by the compositor's hit-test data, which a cover counts in once it has
    // been painted: the cover is in place, painted, before the input.
    *rig.engine.after_point_validation.lock().unwrap() = Some("new Promise(resolve=>{document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;inset:0\">Parent cover</button>');requestAnimationFrame(()=>requestAnimationFrame(resolve))})".into());
    let action = vw_browser::parse(
        &serde_json::json!({"action":"click","ref":item.id,"x":b.x+b.width/2,"y":b.y+b.height/2}),
    )
    .unwrap();
    let error = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await
        .unwrap_err();
    assert!(
        error
            .text
            .contains("the click landed on button Parent cover"),
        "{error:?}"
    );
    assert!(error.text.contains("receipt, frame"), "{error:?}");
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unarmed_confirmation_refuses_before_native_input() {
    let (rig, id) = button_in_world(true).await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    rig.service.evaluate_frames_on_mount(&mount, None,
        "window.addEventListener = () => { throw new Error('fixture cannot arm') }; JSON.stringify(true)".into(), None).await.unwrap();
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(error.outcome, vw_browser::Outcome::Refused);
}

#[tokio::test(flavor = "multi_thread")]
async fn receipt_does_not_disclose_a_target_in_an_unreadable_frame() {
    let (rig, id) = button_in_world(true).await;
    rig.engine
        .unreadable_receipt
        .store(true, std::sync::atomic::Ordering::Relaxed);
    *rig.engine.after_point_validation.lock().unwrap() = Some("document.body.insertAdjacentHTML('beforeend','<button style=\"position:fixed;left:20px;top:20px;width:100px;height:40px\">Private label</button>')".into());
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::ClickAt {
                reference: id,
                x: 50,
                y: 40,
            },
            Actor::User,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(!error.text.contains("Private label"), "{error:?}");
    assert_eq!(error.outcome, vw_browser::Outcome::Failed);
}

/// A focused read can return a visible control that an unfiltered read of the same dense page lists past its ref
/// cap. A coordinate click on that ref still finds its own node in the click's fresh observation and goes ahead,
/// checked and receipt-confirmed, while nothing on the page changed.
#[tokio::test(flavor = "multi_thread")]
async fn a_focused_ref_past_the_ref_cap_of_a_dense_page_can_be_clicked_by_coordinates() {
    let rig = Rig::new().await;
    rig.engine
        .isolate_scripts
        .store(true, std::sync::atomic::Ordering::Relaxed);
    rig.engine.eval(r##"(() => {
        document.body.style = 'margin:0';
        document.body.innerHTML = Array.from({length:260}, (_, i) =>
          `<button style="position:absolute;left:${(i % 25) * 12}px;top:${Math.floor(i / 25) * 10}px;width:10px;height:8px;padding:0;border:0">${i}</button>`).join('') +
          '<a id="late" href="#late" onclick="event.preventDefault();document.body.dataset.clicked=this.id" style="position:absolute;left:20px;top:150px;width:100px;height:30px;display:block">Late link</a>';
        scrollTo(0, 0);
    })()"##).await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                find: Some("Late link".into()),
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    let late = model
        .refs
        .iter()
        .find(|r| r.name == "Late link")
        .expect("focused late ref");
    let bounds = late.bounds.expect("late link bounds");
    let action = vw_browser::Action::ClickAt {
        reference: late.id.clone(),
        x: u32::try_from(bounds.x + bounds.width / 2).unwrap(),
        y: u32::try_from(bounds.y + bounds.height / 2).unwrap(),
    };
    let result = rig
        .browser
        .act(SESSION, action, Actor::User, &NoNavigation)
        .await;
    assert_eq!(
        rig.engine
            .native_clicks
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "{result:?}"
    );
    result.expect("a coordinate click on an unchanged focused ref was refused");
    assert_eq!(
        rig.engine.eval("document.body.dataset.clicked").await,
        serde_json::json!("late")
    );
}
