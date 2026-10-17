//! Capture manifests are required commit records; failure never publishes an unusable Before.
//! Depends on the real page fixture and filesystem fault injection; called by cargo test.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::{NoFetch, Rig, SESSION};
use vw_browser::capture::{Target, parse};
use vw_browser::{ActError, Actor, Outcome};

async fn capture(rig: &Rig, what: &str) -> Result<Value, ActError> {
    rig.browser
        .capture(
            SESSION,
            parse(&json!({"what":what, "shotMethod":"stitch"})).unwrap(),
            Actor::User,
            Target {
                session_folder: &rig.dir,
                fetch: &NoFetch,
            },
        )
        .await
}

fn block_new_manifests(root: &Path) {
    for entry in std::fs::read_dir(root.join("capture")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && !path.join("capture.json").exists() {
            std::fs::create_dir(path.join("capture.json")).unwrap();
        }
    }
}

#[tokio::test]
async fn before_manifest_failure_is_explicit_and_after_refuses() {
    let rig = Rig::new().await;
    let root = rig.dir.clone();
    *rig.engine.on_pin.lock().unwrap() = Some(Box::new(move || block_new_manifests(&root)));
    let error = capture(&rig, "before")
        .await
        .expect_err("the Before manifest is required");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("capture.json"), "{error:?}");
    assert!(
        std::fs::read_dir(rig.dir.join("capture"))
            .unwrap()
            .any(|e| e.unwrap().path().join("before.png").is_file())
    );
    let after = capture(&rig, "after").await.unwrap_err();
    assert_eq!(after.outcome, Outcome::Refused);
    assert!(after.text.contains("there is no before shot"), "{after:?}");
}

#[tokio::test]
async fn failed_new_before_keeps_the_previous_committed_before() {
    let rig = Rig::new().await;
    let before = capture(&rig, "before").await.unwrap();
    let root = rig.dir.clone();
    *rig.engine.on_pin.lock().unwrap() = Some(Box::new(move || block_new_manifests(&root)));
    capture(&rig, "before")
        .await
        .expect_err("failed replacement must not become Before");
    let after = capture(&rig, "after").await.unwrap();
    assert_eq!(before["folder"], after["folder"]);
    for file in after["files"].as_array().unwrap() {
        assert!(
            rig.dir
                .join(after["folder"].as_str().unwrap())
                .join(file["name"].as_str().unwrap())
                .is_file()
        );
    }
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(
            rig.dir
                .join(after["folder"].as_str().unwrap())
                .join("capture.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["what"], "after");
    assert!(manifest["summary"]["before"].is_object());
}

#[tokio::test]
async fn after_manifest_failure_is_explicit_and_before_is_still_available() {
    let rig = Rig::new().await;
    let before = capture(&rig, "before").await.unwrap();
    let folder = rig.dir.join(before["folder"].as_str().unwrap());
    let original = std::fs::read(folder.join("capture.json")).unwrap();
    std::fs::create_dir(folder.join("capture.json.tmp")).unwrap();
    let error = capture(&rig, "after")
        .await
        .expect_err("the After manifest is required");
    assert_eq!(error.outcome, Outcome::Failed);
    assert!(error.text.contains("capture.json"), "{error:?}");
    assert_eq!(
        std::fs::read(folder.join("capture.json")).unwrap(),
        original
    );
    // Remove the injected obstacle and the uncommitted output so a manual retry can commit normally.
    std::fs::remove_dir(folder.join("capture.json.tmp")).unwrap();
    for name in ["after.png", "diff.png", "compare.json"] {
        std::fs::remove_file(folder.join(name)).unwrap();
    }
    assert_eq!(
        capture(&rig, "after").await.unwrap()["folder"],
        before["folder"]
    );
}

#[tokio::test]
async fn after_refuses_a_before_whose_manifest_was_removed() {
    let rig = Rig::new().await;
    let before = capture(&rig, "before").await.unwrap();
    let folder = rig.dir.join(before["folder"].as_str().unwrap());
    std::fs::remove_file(folder.join("capture.json")).unwrap();
    let error = capture(&rig, "after").await.unwrap_err();
    assert_eq!(error.outcome, Outcome::Refused);
    assert!(error.text.contains("Before capture record"), "{error:?}");
    assert!(error.text.contains("capture Before again"), "{error:?}");
    assert!(!folder.join("after.png").exists());
}
