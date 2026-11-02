//! Frame ref acceptance through the browser service and real cross-origin Chromium documents.
//! Uses the shared page engine; frame identities must expire on navigation and vault scope follows the field.
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

async fn framed() -> Rig {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML = '<iframe style=\"position:fixed;left:12px;top:8px;width:280px;height:180px\" src=\"https://frame.example.test/login\"></iframe>'; scrollTo(0,0)").await;
    rig.engine
        .wait_for_frame("https://frame.example.test/login", "#password")
        .await;
    rig.engine
        .wait_for_frame("https://nested.example.test/child", "#nested")
        .await;
    rig
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_and_nested_controls_are_ordinary_refs() {
    let rig = framed().await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(
        model.refs.iter().any(|r| r.name == "Fixture checkbox"),
        "cross-origin checkbox missing: {model:?}"
    );
    assert!(
        model.refs.iter().any(|r| r.name == "Nested button"),
        "nested frame missing"
    );
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .unwrap();
    let target = rig
        .browser
        .fill_check(SESSION, &field.id, Actor::User)
        .await
        .unwrap();
    assert_eq!(target.origin, "https://frame.example.test");
    assert_eq!(target.host, "frame.example.test");
    rig.browser
        .fill_write(SESSION, &target, "fixture-password", Actor::User)
        .await
        .unwrap();
    let checkbox = model
        .refs
        .iter()
        .find(|r| r.name == "Fixture checkbox")
        .unwrap();
    assert!(checkbox.frame_id.is_some());
    assert!(checkbox.bounds.unwrap().x >= 12);
    let receipt = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::Check {
                reference: checkbox.id.clone(),
                checked: true,
            },
            Actor::User,
            &NoNavigation,
        )
        .await;
    assert!(receipt.is_ok(), "frame checkbox: {receipt:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigated_frame_cannot_receive_an_observed_login() {
    let rig = framed().await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .expect("cross-origin password ref");
    let target = rig
        .browser
        .fill_check(SESSION, &field.id, Actor::User)
        .await
        .unwrap();
    rig.engine
        .eval("document.querySelector('iframe').src = 'https://frame.example.test/replaced'")
        .await;
    rig.engine
        .wait_for_frame("https://frame.example.test/replaced", "#password")
        .await;
    assert!(
        rig.browser
            .fill_write(SESSION, &target, "fixture-password", Actor::User)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn fill_rechecks_field_after_beforeinput() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<form method=post><input id=password type=password></form>'; document.querySelector('input').addEventListener('beforeinput', e => e.target.type='text')").await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .unwrap();
    let target = rig
        .browser
        .fill_check(SESSION, &field.id, Actor::User)
        .await
        .unwrap();
    let result = rig
        .browser
        .fill_write(SESSION, &target, "fixture-password", Actor::User)
        .await;
    assert!(
        result.is_err(),
        "field changed to clear text during beforeinput"
    );
    assert_eq!(
        rig.engine
            .eval("document.querySelector('input').value")
            .await,
        ""
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_addresses_do_not_expose_query_credentials() {
    let rig = framed().await;
    rig.engine
        .eval("document.querySelector('iframe').src += '?token=frame-query-value-never-shown'")
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (answer, _) = rig
        .browser
        .read(
            SESSION,
            ReadOptions {
                max_bytes: 64000,
                ..ReadOptions::default()
            },
            false,
            Actor::User,
        )
        .await
        .unwrap();
    assert!(!answer.to_string().contains("frame-query-value-never-shown"));
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_frame_does_not_break_parent_read_or_expose_visible_controls() {
    let rig = framed().await;
    rig.engine
        .eval("document.querySelector('iframe').style.display='none'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(
        model
            .refs
            .iter()
            .filter(|r| r.frame == Some(true))
            .all(|r| r.visible == Some(false))
    );
    assert!(!model.text.contains("Fixture checkbox"));
}

#[tokio::test(flavor = "multi_thread")]
async fn main_frame_refs_still_work_with_user_annotations() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<button id=annotated>Annotation target</button>'")
        .await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let item = model
        .refs
        .iter()
        .find(|r| r.name == "Annotation target")
        .unwrap();
    let page = rig.service.page_accessor(SESSION).unwrap();
    let annotation = vw_browser::annotate::annotate(
        &rig.service,
        &page,
        SESSION,
        false,
        Some(&serde_json::json!({"ref":item.id})),
    )
    .await
    .unwrap();
    assert_eq!(annotation.name, "Annotation target");
}

#[tokio::test(flavor = "multi_thread")]
async fn same_origin_frame_uses_its_own_document_and_preserves_annotation_refs() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<iframe src=\"/same-frame\"></iframe>'")
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .unwrap();
    let target = rig
        .browser
        .fill_check(SESSION, &field.id, Actor::User)
        .await
        .unwrap();
    assert_eq!(target.origin, "https://93.184.215.14");
    rig.browser
        .fill_write(SESSION, &target, "fixture-value", Actor::User)
        .await
        .unwrap();
    let page = rig.service.page_accessor(SESSION).unwrap();
    let annotation = vw_browser::annotate::annotate(
        &rig.service,
        &page,
        SESSION,
        false,
        Some(&serde_json::json!({"ref":field.id})),
    )
    .await
    .unwrap();
    assert_eq!(annotation.name, "Password");
}

#[tokio::test(flavor = "multi_thread")]
async fn merged_frame_refs_obey_the_requested_limit() {
    let rig = framed().await;
    let (model, _, _) = rig
        .browser
        .observe(
            SESSION,
            ReadOptions {
                max_refs: 1,
                ..ReadOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(model.refs.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_cannot_claim_another_origin_in_its_observation() {
    let rig = framed().await;
    let mount = rig.service.page_mount(SESSION).unwrap();
    rig.service.evaluate_frames_on_mount(&mount, None,
        "if(location.hostname==='frame.example.test'){const stringify=JSON.stringify;JSON.stringify=function(value,...args){if(value?.refs && value.url)value.url='https://93.184.215.14/';return stringify.call(this,value,...args)}};JSON.stringify(true)".into(), None).await.unwrap();
    // The forged frame is left out (nothing of it to act on or fill into); the rest of the page is still read.
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(
        model
            .refs
            .iter()
            .all(|r| r.name != "Fixture checkbox" && r.login.is_none()),
        "frame claimed the parent's vault origin: {:?}",
        model.refs
    );
    assert_eq!(
        model.unread_frames,
        ["a frame's address or origin could not be checked"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_sandbox_origin_cannot_receive_a_vault_fill() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<iframe sandbox=\"allow-scripts\" src=\"https://frame.example.test/login\"></iframe>'").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .unwrap();
    assert!(
        rig.browser
            .fill_check(SESSION, &field.id, Actor::User)
            .await
            .is_err(),
        "opaque origin accepted a site credential"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn inline_frame_controls_are_read_and_filled_with_the_parents_origin() {
    let rig = Rig::new().await;
    rig.engine.eval("const f=document.createElement('iframe');f.srcdoc='<form method=post><input type=password aria-label=Secret><button>Inline control</button></form>';document.body.replaceChildren(f)").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert!(model.refs.iter().any(|r| r.name == "Inline control"));
    let field = model
        .refs
        .iter()
        .find(|r| r.login.as_deref() == Some("password"))
        .unwrap();
    let target = rig
        .browser
        .fill_check(SESSION, &field.id, Actor::User)
        .await
        .unwrap_or_else(|e| panic!("{}", e.text));
    assert_eq!(target.origin, "https://93.184.215.14");
}

#[tokio::test(flavor = "multi_thread")]
async fn sandbox_cannot_replace_window_origin_to_request_a_site_login() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<iframe sandbox=\"allow-scripts\" src=\"https://frame.example.test/login\"></iframe>'").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // The frame's own page script; a page-world evaluator shares it, an isolated world does not.
    rig.engine
        .eval_in_frame_pages("if(location.hostname==='frame.example.test')Object.defineProperty(window,'origin',{value:'https://frame.example.test'})")
        .await;
    if let Ok((model, _, _)) = rig.browser.observe(SESSION, ReadOptions::default()).await
        && let Some(field) = model
            .refs
            .iter()
            .find(|r| r.login.as_deref() == Some("password"))
    {
        assert!(
            rig.browser
                .fill_check(SESSION, &field.id, Actor::User)
                .await
                .is_err(),
            "page-supplied origin granted a vault target"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_reply_order_does_not_change_ref_order_or_snapshot() {
    let rig = framed().await;
    let (first, _, snapshot) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    rig.engine
        .reverse_frames
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (second, _, again) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(
        first.refs.iter().map(|r| &r.id).collect::<Vec<_>>(),
        second.refs.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    assert_eq!(snapshot, again);
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_to_a_control_in_another_sites_frame_below_the_screen_shows_it() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.style.margin='0'; document.body.innerHTML = '<div style=\"height:900px\"></div><iframe style=\"width:280px;height:180px;border:0\" src=\"https://frame.example.test/login\"></iframe><div style=\"height:900px\"></div>'; scrollTo(0,0)").await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let (model, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let checkbox = model
        .refs
        .iter()
        .find(|r| r.name == "Fixture checkbox")
        .unwrap_or_else(|| panic!("cross-origin checkbox missing: {model:?}"));
    let scrolled = rig
        .browser
        .act(
            SESSION,
            vw_browser::Action::Scroll {
                reference: Some(checkbox.id.clone()),
                up: false,
                amount: None,
            },
            Actor::Agent,
            &NoNavigation,
        )
        .await;
    let (after, _, _) = rig
        .browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let now = after
        .refs
        .iter()
        .find(|r| r.name == "Fixture checkbox")
        .and_then(|r| r.bounds)
        .unwrap();
    let height = i32::try_from(after.viewport.unwrap().1).unwrap();
    // Done means it shows; otherwise the answer must say the page did not move.
    match scrolled {
        Ok(_) => assert!(
            now.y >= 0 && now.y + now.height <= height,
            "scroll answered done but the control is off screen: {now:?}"
        ),
        Err(error) => assert!(error.text.contains("did not move"), "{error:?}"),
    }
}
