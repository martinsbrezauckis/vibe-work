//! Screenshot receipts bind CSS coordinates to the actual PNG dimensions at native device scales, and an area
//! capture is taken of one unchanged page: a view resized once during it (the app's strip appearing as the action
//! starts) is captured again, a view that keeps resizing or a page that moved to another address is refused.
//! Uses the real browser driver and Chromium pixels; never navigates outside the fixture origin.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use support::screenshot::AfterShot;
use support::{Rig, SESSION};
use vw_browser::Actor;

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

const CLIP: &str =
    r#"{"action":"capture.screenshot","clip":{"x":0,"y":0,"width":120,"height":80}}"#;

async fn capture_clip(rig: &Rig) -> Result<serde_json::Value, String> {
    let request = serde_json::from_str(CLIP).unwrap();
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&request).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .map_err(|e| format!("{e:?}"))
}

// PREDICT (before the fix): refused "The page or capture element changed during the screenshot": the readings
// before and after the one capture name different viewport heights (200 / 152), although the page never changed.
#[tokio::test]
async fn a_view_resized_once_during_an_area_capture_is_captured_again() {
    let rig = Rig::new().await;
    rig.engine.viewport(300, 200).await;
    rig.engine.after_shots([AfterShot::Resize(300, 152)]);
    let value = capture_clip(&rig).await.unwrap();
    assert_eq!(
        value["cssRect"],
        json!({"x":0,"y":0,"width":120,"height":80})
    );
    assert_eq!(value["pixelSize"], json!({"width":120,"height":80}));
    assert_eq!(rig.engine.after_shots_left(), 0);
}

#[tokio::test]
async fn a_view_that_keeps_resizing_during_area_captures_is_refused() {
    let rig = Rig::new().await;
    rig.engine.viewport(300, 200).await;
    rig.engine.after_shots([
        AfterShot::Resize(300, 152),
        AfterShot::Resize(300, 200),
        AfterShot::Resize(300, 152),
        AfterShot::Resize(300, 200),
    ]);
    let refusal = capture_clip(&rig).await.unwrap_err();
    assert!(
        refusal.contains("changed during the screenshot"),
        "{refusal}"
    );
    // Every capture saw a resize; the refusal comes after a bounded number of them, not on the first.
    assert_eq!(rig.engine.after_shots_left(), 1);
}

#[tokio::test]
async fn an_address_change_during_an_area_capture_is_refused_without_capturing_again() {
    let rig = Rig::new().await;
    rig.engine.viewport(300, 200).await;
    rig.engine.after_shots([
        AfterShot::Eval("history.pushState(null, '', '/moved')"),
        AfterShot::Resize(300, 152),
    ]);
    let refusal = capture_clip(&rig).await.unwrap_err();
    assert!(
        refusal.contains("changed during the screenshot"),
        "{refusal}"
    );
    assert_eq!(rig.engine.after_shots_left(), 1);
}

#[tokio::test]
async fn rectangle_receipt_names_css_rect_and_device_pixels_at_each_scale() {
    let rig = Rig::new().await;
    for scale in [1.0, 1.5, 2.0] {
        rig.engine.scaled_viewport(300, 200, scale).await;
        let clip = json!({"x":10,"y":20,"width":120,"height":80});
        let value = rig
            .browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"capture.screenshot","clip":clip})).unwrap(),
                Actor::Agent,
                &NoNavigation,
            )
            .await
            .unwrap();
        let png = STANDARD
            .decode(value["vwImage"]["data"].as_str().unwrap())
            .unwrap();
        let image = vw_preview::decode(&png).unwrap();
        assert_eq!(f64::from(image.width), 120.0 * scale);
        assert_eq!(f64::from(image.height), 80.0 * scale);
        assert_eq!(value["width"], image.width);
        assert_eq!(value["height"], image.height);
        assert_eq!(value["cssRect"], clip);
        assert_eq!(
            value["pixelSize"],
            json!({"width":image.width,"height":image.height})
        );
        assert_eq!(value["scale"], json!({"x":scale,"y":scale}));
    }
}
