//! Full-page cleanup after Stop, timeout, errors and completion on a real fixture page.
//! Depends on support's Chromium engine; called by cargo test. Cancellation happens after real styles changed.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;
use support::{NoFetch, Rig, SESSION};
use vw_browser::capture::{
    Target, parse,
    shot::{self, Method},
};
use vw_browser::{Actor, Outcome};

async fn restored(rig: &Rig, original: &serde_json::Value) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if rig.engine.snapshot().await == *original {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("capture must restore styles and both scroll coordinates");
}

#[tokio::test]
async fn stop_restores_stitched_page_after_pins_were_hidden() {
    let rig = Rig::new().await;
    let original = rig.engine.snapshot().await;
    rig.engine.hold_pin.store(true, Ordering::Relaxed);
    let work = rig.browser.capture(
        SESSION,
        parse(&json!({"what":"before", "shotMethod":"stitch"})).unwrap(),
        Actor::Agent,
        Target {
            session_folder: &rig.dir,
            fetch: &NoFetch,
        },
    );
    tokio::pin!(work);
    tokio::select! { () = rig.engine.pinned.notified() => {}, result = &mut work => panic!("finished before cancellation: {result:?}") }
    assert_ne!(rig.engine.snapshot().await, original);
    rig.browser.sessions().stop(SESSION);
    assert_eq!(work.await.unwrap_err().outcome, Outcome::Stopped);
    restored(&rig, &original).await;
}

#[tokio::test]
async fn capture_timeout_restores_stitched_page_after_pins_were_hidden() {
    let rig = Rig::new().await;
    let original = rig.engine.snapshot().await;
    rig.engine.hold_pin.store(true, Ordering::Relaxed);
    let work = rig.browser.capture(
        SESSION,
        parse(&json!({"what":"before", "shotMethod":"stitch"})).unwrap(),
        Actor::User,
        Target {
            session_folder: &rig.dir,
            fetch: &NoFetch,
        },
    );
    tokio::pin!(work);
    tokio::select! { () = rig.engine.pinned.notified() => {}, result = &mut work => panic!("finished before timeout: {result:?}") }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(46)).await;
    let error = work.await.unwrap_err();
    tokio::time::resume();
    assert!(error.text.contains("took too long"), "{error:?}");
    restored(&rig, &original).await;
}

#[tokio::test]
async fn normal_completion_and_slice_failure_restore_the_page() {
    for fail in [false, true] {
        let rig = Rig::new().await;
        let original = rig.engine.snapshot().await;
        rig.engine.fail_pin.store(fail, Ordering::Relaxed);
        let result = shot::full_page(&rig.browser, SESSION, Some(Method::Stitch)).await;
        assert_eq!(result.is_err(), fail, "{result:?}");
        assert_eq!(rig.engine.snapshot().await, original);
    }
}
