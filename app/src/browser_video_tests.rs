//! Tests the trusted encoder channel across main-document replacement using real channel delivery.
use super::*;
use std::sync::atomic::AtomicUsize;
use tauri::ipc::InvokeResponseBody;

fn channel(video: &BrowserVideo, deliveries: Arc<AtomicUsize>) -> Channel<UiRequest> {
    channel_with_frame_error(video, deliveries, false)
}

fn channel_with_frame_error(
    video: &BrowserVideo,
    deliveries: Arc<AtomicUsize>,
    fail_frame: bool,
) -> Channel<UiRequest> {
    let inner = Arc::downgrade(&video.0);
    Channel::new(move |body| {
        deliveries.fetch_add(1, Ordering::SeqCst);
        let InvokeResponseBody::Json(body) = body else {
            return Ok(());
        };
        let value: serde_json::Value = serde_json::from_str(&body)?;
        let Some(inner) = inner.upgrade() else {
            return Ok(());
        };
        let answer = UiReply {
            id: value["id"].as_u64().unwrap_or_default(),
            ok: !(fail_frame && value["kind"] == "frame"),
            supported: Some(true),
            bytes: Some(base64::engine::general_purpose::STANDARD.encode(b"\x1a\x45\xdf\xa3mock")),
            frames: Some(1),
            duration_ms: Some(250),
            error: None,
        };
        if let Some(sender) = inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&answer.id)
        {
            let _ = sender.send(answer);
        }
        Ok(())
    })
}

// PREDICT: replacing the main document between encoder calls still lets the old recording
// deliver a frame or finish to the replacement renderer instead of discarding its bytes.
#[tokio::test]
async fn main_document_replacement_discards_old_video_without_delivering_to_the_new_encoder() {
    for finish in [false, true] {
        let video = BrowserVideo::default();
        video.replace_channel(channel(&video, Arc::new(AtomicUsize::new(0))));
        assert!(matches!(
            video
                .send(VideoCommand::Start {
                    recording: "old".into()
                })
                .await,
            Ok(VideoReply::Ready)
        ));
        let replacement = Arc::new(AtomicUsize::new(0));
        video.replace_channel(channel(&video, replacement.clone()));
        let request = if finish {
            VideoCommand::Finish {
                recording: "old".into(),
                at_seconds: 0.5,
            }
        } else {
            VideoCommand::Frame {
                recording: "old".into(),
                png: vec![1],
                at_seconds: 0.25,
            }
        };
        assert!(
            video.send(request).await.is_err(),
            "the replaced document's recording must be discarded"
        );
        assert_eq!(replacement.load(Ordering::SeqCst), 0);
        assert!(matches!(
            video
                .send(VideoCommand::Start {
                    recording: "new".into()
                })
                .await,
            Ok(VideoReply::Ready)
        ));
        assert!(matches!(
            video
                .send(VideoCommand::Frame {
                    recording: "new".into(),
                    png: vec![1],
                    at_seconds: 0.25
                })
                .await,
            Ok(VideoReply::Added)
        ));
    }
}

// A rejected frame still needs cancellation on its original encoder to release the recording.
#[tokio::test]
async fn frame_error_keeps_the_original_encoder_reachable_for_cancellation() {
    let video = BrowserVideo::default();
    let deliveries = Arc::new(AtomicUsize::new(0));
    video.replace_channel(channel_with_frame_error(&video, deliveries.clone(), true));
    assert!(matches!(
        video
            .send(VideoCommand::Start {
                recording: "r".into()
            })
            .await,
        Ok(VideoReply::Ready)
    ));
    assert!(
        video
            .send(VideoCommand::Frame {
                recording: "r".into(),
                png: vec![1],
                at_seconds: 0.25
            })
            .await
            .is_err()
    );
    assert!(matches!(
        video
            .send(VideoCommand::Cancel {
                recording: "r".into()
            })
            .await,
        Ok(VideoReply::Added)
    ));
    assert_eq!(deliveries.load(Ordering::SeqCst), 3);
}
