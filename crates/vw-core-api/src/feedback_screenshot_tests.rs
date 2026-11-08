//! Feedback captures are native-only, optional and immutable until the person changes their choice.
use crate::{Command, ErrorCode, FeedbackDraft, FeedbackKind, FeedbackRequest as R, Origin};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn draft() -> FeedbackDraft {
    FeedbackDraft {
        kind: FeedbackKind::Bug,
        title: "Missing tab".into(),
        description: "Close a tab.".into(),
        remarks: String::new(),
    }
}
#[tokio::test]
async fn feedback_screenshot_requires_a_person_and_fresh_opt_in() {
    let core = crate::offline_core::start("feedback-capture").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    core.set_feedback_capture(move || {
        captured.fetch_add(1, Ordering::SeqCst);
        async { vw_preview::encode(&vw_preview::canvas(40, 20, [128, 128, 128, 255])) }
    });
    core.feedback(Origin::DebugApi, R::Open { draft: draft() })
        .await
        .unwrap();
    assert_eq!(
        core.feedback(Origin::DebugApi, R::Screenshot { include: true })
            .await
            .unwrap_err()
            .code,
        ErrorCode::Denied
    );
    core.set_window_drive(|| true);
    assert_eq!(
        core.feedback(Origin::Ui, R::Screenshot { include: true })
            .await
            .unwrap_err()
            .code,
        ErrorCode::Denied
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    core.shutdown().await;
}
#[tokio::test]
async fn feedback_attaches_only_the_captured_image_and_reuses_identical_review_ids() {
    let core = crate::offline_core::start("feedback-image-review").await;
    core.set_feedback_capture(|| async {
        vw_preview::encode(&vw_preview::canvas(40, 20, [128, 128, 128, 255]))
    });
    let opened = core
        .feedback(Origin::Ui, R::Open { draft: draft() })
        .await
        .unwrap();
    assert!(!opened.add_screenshot);
    assert!(opened.screenshot.is_none());
    let captured = core
        .feedback(Origin::Ui, R::Screenshot { include: true })
        .await
        .unwrap();
    let image = captured.screenshot.unwrap();
    assert_eq!(
        (image.width, image.height, image.mime_type.as_str()),
        (40, 20, "image/png")
    );
    let first = core
        .feedback(Origin::Ui, R::Review { draft: draft() })
        .await
        .unwrap()
        .review
        .unwrap();
    let second = core
        .feedback(Origin::Ui, R::Review { draft: draft() })
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.screenshot, Some(image));
    core.feedback(Origin::Ui, R::Screenshot { include: false })
        .await
        .unwrap();
    let omitted = core
        .feedback(Origin::Ui, R::Review { draft: draft() })
        .await
        .unwrap()
        .review
        .unwrap();
    assert!(omitted.screenshot.is_none());
    assert!(
        !serde_json::to_value(&omitted)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("screenshot")
    );
    assert_ne!(omitted.id, first.id);
    core.feedback(Origin::Ui, R::Close).await.unwrap();
    let next = core
        .feedback(Origin::Ui, R::Open { draft: draft() })
        .await
        .unwrap();
    assert!(!next.add_screenshot);
    assert!(next.screenshot.is_none());
    core.shutdown().await;
}
#[tokio::test]
async fn feedback_failed_secret_checks_and_invalid_native_images_attach_nothing() {
    for (index, kind) in ["missing", "secret", "bytes", "large", "dimensions"]
        .into_iter()
        .enumerate()
    {
        let core = crate::offline_core::start(&format!("feedback-image-invalid-{index}")).await;
        match kind {
            "missing" => {}
            "secret" => core.set_feedback_capture(|| async {
                Err("Secret fields could not be hidden. No screenshot attached.".into())
            }),
            "bytes" => core.set_feedback_capture(|| async { Ok(b"not a PNG".to_vec()) }),
            "large" => core.set_feedback_capture(|| async { Ok(vec![0; 1_048_577]) }),
            _ => core.set_feedback_capture(|| async {
                vw_preview::encode(&vw_preview::canvas(4097, 1, [0, 0, 0, 255]))
            }),
        }
        core.feedback(Origin::Ui, R::Open { draft: draft() })
            .await
            .unwrap();
        let view = core
            .feedback(Origin::Ui, R::Screenshot { include: true })
            .await
            .unwrap();
        assert!(view.add_screenshot);
        assert!(view.screenshot.is_none());
        assert!(
            view.screenshot_error
                .unwrap()
                .contains("No screenshot attached")
        );
        let report = core
            .feedback(Origin::Ui, R::Review { draft: draft() })
            .await
            .unwrap()
            .review
            .unwrap();
        assert!(report.screenshot.is_none());
        core.shutdown().await;
    }
}
#[test]
fn feedback_callers_cannot_supply_image_bytes_files_or_diagnostics() {
    for field in ["screenshot", "path", "dataBase64", "system"] {
        let mut request = serde_json::json!({"op":"screenshot", "include":true});
        request[field] = serde_json::json!("caller-data");
        assert!(
            serde_json::from_value::<Command>(
                serde_json::json!({"type":"feedback", "request":request})
            )
            .is_err()
        );
    }
}
