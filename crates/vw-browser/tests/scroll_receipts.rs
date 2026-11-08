//! Directional scroll receipts through the production driver on a smooth-scrolling page.
//! Uses the real Chromium fixture; the receipt must describe the completed movement without replaying input.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::{Actor, ReadOptions};

struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}

async fn scroll(rig: &Rig, direction: &str, amount: u32) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"scroll","direction":direction,"amount":amount}))
                .unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn directional_scroll_receipts_include_the_completed_movement_on_smooth_pages() {
    let rig = Rig::new().await;
    rig.engine.eval("document.documentElement.style.scrollBehavior='smooth';document.body.innerHTML='<main style=height:6000px>Scrollable page</main>';scrollTo({top:0,behavior:'instant'})").await;
    rig.browser
        .observe(SESSION, ReadOptions::default())
        .await
        .unwrap();
    let mut previous = 0;
    for (direction, amount, expected) in [("down", 600, 600), ("up", 250, 350), ("up", 600, 0)] {
        let receipt = scroll(&rig, direction, amount).await;
        assert_eq!(receipt["result"]["scrollY"], expected, "{receipt}");
        assert!(
            receipt["change"]["summary"]
                .as_str()
                .unwrap()
                .contains("scrolled"),
            "{receipt}"
        );
        assert_eq!(receipt["change"]["scroll"]["from"], previous, "{receipt}");
        assert_eq!(receipt["change"]["scroll"]["to"], expected, "{receipt}");
        previous = expected;
        assert_eq!(rig.engine.eval("Math.round(scrollY)").await, expected);
        let (model, _, _) = rig
            .browser
            .observe(SESSION, ReadOptions::default())
            .await
            .unwrap();
        assert_eq!(model.scroll.y, expected);
    }
}
