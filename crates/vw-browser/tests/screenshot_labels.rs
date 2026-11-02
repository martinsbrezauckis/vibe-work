//! Agent ref labels belong only to the returned image, never to the live document or ordinary captures.
//! Compares real screenshot pixels and page markup through the shared fixture engine.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

#[tokio::test(flavor = "multi_thread")]
async fn agent_copy_has_labels_and_page_copy_stays_clean() {
    let rig = Rig::new().await;
    rig.engine.eval("document.body.innerHTML='<button style=\"position:fixed;left:20px;top:40px\">Save</button>'; scrollTo(0,0)").await;
    let markup = rig.engine.eval("document.documentElement.outerHTML").await;
    let raw = rig.service.screenshot(SESSION).await.unwrap();
    let (_, shot) = rig
        .browser
        .read(SESSION, ReadOptions::default(), true, Actor::User)
        .await
        .unwrap();
    let pixels = vw_preview::decode(&shot.unwrap().png).unwrap();
    assert_ne!(
        pixels,
        vw_preview::decode(&raw.png).unwrap(),
        "agent screenshot has no ref labels"
    );
    assert_eq!(
        markup,
        rig.engine.eval("document.documentElement.outerHTML").await
    );
    assert_eq!(
        vw_preview::decode(&raw.png).unwrap(),
        vw_preview::decode(&rig.service.screenshot(SESSION).await.unwrap().png).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_during_capture_cannot_return_labels_for_the_old_document() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<button>Before navigation</button>'")
        .await;
    rig.engine
        .navigate_during_shot
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        rig.browser
            .read(SESSION, ReadOptions::default(), true, Actor::Agent)
            .await
            .is_err()
    );
}

/// A focused read with a screenshot names the CSS viewport as a whole read does: the labelled image alone does not
/// say how its pixels map to the x/y a click takes (a Retina screen draws two pixels per CSS pixel).
#[tokio::test(flavor = "multi_thread")]
async fn a_focused_read_with_a_screenshot_names_the_viewport() {
    let rig = Rig::new().await;
    rig.engine
        .eval("document.body.innerHTML='<label><input type=checkbox> Real hand</label><button>Other</button>'")
        .await;
    let (whole, _) = rig
        .browser
        .read(SESSION, ReadOptions::default(), true, Actor::Agent)
        .await
        .unwrap();
    let options = ReadOptions {
        find: Some("Real hand".into()),
        ..ReadOptions::default()
    };
    let (found, shot) = rig
        .browser
        .read(SESSION, options, true, Actor::Agent)
        .await
        .unwrap();
    assert!(shot.is_some());
    let viewport = &whole["untrustedPageContent"]["viewport"];
    assert!(viewport["width"].as_u64().is_some(), "{whole}");
    assert_eq!(
        &found["untrustedPageContent"]["viewport"], viewport,
        "{found}"
    );
    assert!(
        found["untrustedPageContent"]["refs"][0]["label"].is_string(),
        "{found}"
    );
}
