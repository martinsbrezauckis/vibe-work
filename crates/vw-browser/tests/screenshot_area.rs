//! Screenshot receipts bind CSS coordinates to the actual PNG dimensions at native device scales.
//! Uses the real browser driver and Chromium pixels; never navigates outside the fixture origin.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use support::{Rig, SESSION};
use vw_browser::Actor;

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
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
