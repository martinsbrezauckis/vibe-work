//! Action GIF receipts contain actual captures, reject a second stop and stop collecting at the person's Stop.
#![cfg(feature = "e2e-script")]
#![allow(clippy::unwrap_used)]
mod support;
use base64::Engine as _;
use serde_json::{Value, json};
use support::{Rig, SESSION};
use vw_browser::Actor;
struct NoNavigation;
impl vw_browser::Navigate for NoNavigation {
    fn open<'a>(&'a self, _: &'a str, _: &'a str) -> vw_browser::NavFuture<'a> {
        Box::pin(async { Err("unexpected navigation".into()) })
    }
}
async fn act(rig: &Rig, args: Value) -> Value {
    rig.browser
        .act(
            SESSION,
            vw_browser::parse(&args).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn action_gif_contains_captured_frames_and_stop_is_final() {
    let rig = Rig::new().await;
    let started = act(&rig, json!({"action":"recording.start"})).await;
    assert_eq!(started["recording"]["state"], "recording");
    rig.engine
        .eval("document.body.style.background='red'")
        .await;
    act(&rig, json!({"action":"media.read"})).await;
    let stopped = act(&rig, json!({"action":"recording.stop"})).await;
    assert!(stopped["recording"]["frames"].as_u64().unwrap() >= 2);
    assert!(stopped["_vwRecordingGif"].is_string());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(stopped["_vwRecordingGif"].as_str().unwrap())
        .unwrap();
    let mut decoder = gif::DecodeOptions::new();
    decoder.set_color_output(gif::ColorOutput::RGBA);
    let mut reader = decoder.read_info(bytes.as_slice()).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_next_frame().unwrap() {
        frames.push(frame.buffer.to_vec());
    }
    assert_eq!(
        frames.len() as u64,
        stopped["recording"]["frames"].as_u64().unwrap()
    );
    assert_ne!(
        frames[0], frames[1],
        "GIF must contain the changed page pixels"
    );
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stop_discards_the_recording_and_video_offers_the_gif_alternative() {
    let rig = Rig::new().await;
    act(&rig, json!({"action":"recording.start"})).await;
    rig.browser.sessions().stop(SESSION);
    assert!(
        rig.browser
            .act(
                SESSION,
                vw_browser::parse(&json!({"action":"recording.stop"})).unwrap(),
                Actor::Agent,
                &NoNavigation
            )
            .await
            .is_err()
    );
    rig.browser.sessions().resume(SESSION);
    let restarted = act(&rig, json!({"action":"recording.start"})).await;
    assert_eq!(restarted["recording"]["frames"], 1);
    let error = rig
        .browser
        .act(
            SESSION,
            vw_browser::parse(&json!({"action":"recording.video.start"})).unwrap(),
            Actor::Agent,
            &NoNavigation,
        )
        .await
        .unwrap_err();
    assert!(error.text.contains("recording.start") && error.text.contains("encoder"));
}
