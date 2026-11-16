//! The person's "Annotate this page" picture follows the capture rule: below Full access a page showing a password,
//! PIN or card value gives no picture (the note and position stay, and the annotation says why); Full access and
//! a page without protected values give the picture. Real browser driver and Chromium pixels; fake values only.
//!
//! PREDICT (before the admission): the crop of the annotated area is attached on a page showing the typed card
//! number, because `annotate` took its screenshot without asking the capture admission.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use std::sync::Arc;
use support::{Rig, SESSION};
use vw_browser::annotate::{Annotation, annotate};
use vw_browser::protected_shown;

struct Full;
impl vw_browser::delegation::DelegationAuthorizer for Full {
    fn protected_values_delegated(&self, _: &str) -> bool {
        true
    }
}

async fn annotation(rig: &Rig, html: &str) -> Annotation {
    rig.engine
        .eval(&format!("document.body.innerHTML = {}", json!(html)))
        .await;
    let page = rig.service.page_accessor(SESSION).unwrap();
    let target = json!({"rect": {"x": 10, "y": 10, "width": 80, "height": 30}});
    annotate(
        &rig.browser,
        &rig.service,
        &page,
        SESSION,
        true,
        Some(&target),
    )
    .await
    .unwrap()
}

const CARD: &str = "<label>Card number <input value=4111111111119917></label>";

#[tokio::test(flavor = "multi_thread")]
async fn no_picture_below_full_access_while_a_protected_value_shows() {
    let rig = Rig::new().await;
    let shown = annotation(&rig, CARD).await;
    assert!(
        shown.crop_png_base64.is_none(),
        "no picture of the card number"
    );
    assert_eq!(shown.crop_left_out, Some(protected_shown()));
    assert_eq!(shown.mode, "box");
    assert_eq!(shown.rect.width, 80, "the position is kept");
}

#[tokio::test(flavor = "multi_thread")]
async fn full_access_and_ordinary_pages_keep_the_picture() {
    let rig = Rig::new().await;
    let ordinary = annotation(
        &rig,
        "<p>Ordinary page</p><label>Notes <input value=note></label>",
    )
    .await;
    assert!(ordinary.crop_png_base64.is_some());
    assert_eq!(ordinary.crop_left_out, None);
    // The UI's optional `cropLeftOut` is absent, not null, when nothing was left out.
    let wire = serde_json::to_value(&ordinary).unwrap();
    assert!(wire.get("cropLeftOut").is_none(), "{wire}");
    rig.browser.set_delegation_authorizer(Arc::new(Full));
    let full = annotation(&rig, CARD).await;
    assert!(
        full.crop_png_base64.is_some(),
        "delegated session keeps the crop"
    );
    assert_eq!(full.crop_left_out, None);
}
