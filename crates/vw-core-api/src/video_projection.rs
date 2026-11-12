//! Display-only video metadata on Vibe's existing effect rows, enabled only with the video module.
use serde_json::Value;
pub(crate) fn project(value: &mut Value) {
    if value["type"] == "effect" && value["detail"]["toolName"] == "mcp_vw.video_frames" {
        let mut input = value["detail"]["input"].clone();
        if let Some(map) = input.as_object_mut() {
            map.remove(vw_agent_tools::wire::tools::RECEIPT_FIELD);
        }
        if let Ok(request) = serde_json::from_value::<vw_agent_tools::video::Request>(input)
            && let Ok(frames) = request.frame_limit()
        {
            value["vwVideo"] = serde_json::json!({"plannedFrames":frames,"frameLimit":request.scene_limit().is_some(),"imageCount":usize::from(request.output!=vw_agent_tools::video::Output::Metadata),"source":request.path.rsplit(['/', '\\']).next().unwrap_or("Video")});
        }
    }
    match value {
        Value::Object(map) => map.values_mut().for_each(project),
        Value::Array(list) => list.iter_mut().for_each(project),
        _ => {}
    }
}
/// One final boundary for native, worker, request and policy failures. Details remain in the app log.
pub(crate) fn failure(error: &vw_video_types::Error) -> (String, String) {
    tracing::warn!(code = ?error.code, detail = %error.detail, "video extraction failed");
    (error.row(), error.agent())
}
/// Called at all session snapshot/event boundaries when video is enabled.
pub(crate) fn apply(shared: &crate::core::Shared, value: &mut Value) {
    if !vw_agent_tools::video::ENABLED {
        return;
    }
    project(value);
    if let Some(tools) = &shared.agent_tools {
        tools.video_history.overlay(value);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vw_video_types::{Code, Codec, Error, Media};
    #[test]
    fn outcomes_keep_the_specific_diagnosis_without_internal_prefixes() {
        for (code, diagnosis, action) in [
            (Code::Rotation, "rotation", "Ask the user"),
            (Code::Hdr, "HDR", "Ask the user"),
            (Code::WorkerStart, "could not start", "Ask the user"),
            (Code::InvalidReply, "incomplete", "Ask the user"),
            (Code::ScanSamples, "240", "Increase"),
        ] {
            let (_, agent) = failure(&Error::new(code, "backend_unavailable: helper detail"));
            assert!(
                !agent.contains("backend_unavailable:") && !agent.contains("helper detail"),
                "{agent}"
            );
            assert!(agent.contains(diagnosis), "{agent}");
            assert!(agent.contains(action), "{agent}");
        }
    }
    #[test]
    fn diagnostics_cannot_change_the_outcome_or_appear_in_agent_text() {
        for (code, instruction) in [
            (Code::ModelImages, "image-capable model"),
            (Code::Busy, "Wait for an active video job"),
            (Code::InputSize, "smaller video file"),
            (Code::Timeout, "shorter range"),
            (Code::Duration, "no longer than two hours"),
            (Code::Memory, "lower resolution"),
            (Code::ChangedInput, "finished changing"),
            (Code::Cancelled, "next message"),
            (Code::ImageAllowance, "Do not retry in this turn"),
            (Code::WorkerIsolation, "protection"),
            (Code::WorkerFailed, "stopped unexpectedly"),
            (
                Code::UnsupportedCodec {
                    codec: Some(Codec::H264),
                },
                "H.264",
            ),
            (Code::MediaUnavailable { media: Media::Mp4 }, "MP4"),
        ] {
            let expected = failure(&Error::new(code.clone(), "original native reason"));
            let misleading = failure(&Error::new(
                code,
                "helper H.264 image allowance sandbox timeout: not_found: /private/codec",
            ));
            assert_eq!(expected, misleading);
            assert!(expected.1.contains(instruction), "{expected:?}");
            assert!(!expected.1.contains("original native reason"));
        }
    }
    #[test]
    fn automatic_running_effect_has_counts_before_any_result() {
        let mut row = json!({"type":"effect","id":"call","sessionId":"chat","detail":{"toolName":"mcp_vw.video_frames","input":{"path":"clip.mp4","selection":{"kind":"scene_changes","start_ms":1000,"end_ms":4000,"step_ms":250,"max_frames":6},"output":"contact_sheet"}},"state":{"status":"running","approvalSource":"bypass"}});
        project(&mut row);
        assert_eq!(row["vwVideo"]["plannedFrames"], 6);
        assert_eq!(row["vwVideo"]["imageCount"], 1);
        assert_eq!(row["vwVideo"]["frameLimit"], true);
        assert!(row["state"]["output"].is_null());
        row["detail"]["input"]["output"] = json!("metadata");
        project(&mut row);
        assert_eq!(row["vwVideo"]["imageCount"], 0);
    }
}

#[cfg(test)]
mod delivery_tests {
    use crate::feature_test_core::Fixture;
    use serde_json::json;
    #[cfg(all(target_os = "linux", not(feature = "linux-video")))]
    #[tokio::test]
    async fn linux_video_rows_are_not_sent_to_the_window() {
        let f = Fixture::new().await;
        let mut row = json!({"type":"effect","id":"call","sessionId":"s",
            "detail":{"toolName":"mcp_vw.video_frames","input":{"path":"clip.mp4",
            "selection":{"kind":"timestamps","times_ms":[0]},"output":"frame"}}});
        super::apply(&f.core.shared, &mut row);
        assert!(row.get("vwVideo").is_none());
        // Previously saved outcomes must not bring the controls back after a restart either.
        f.core
            .shared
            .agent_tools
            .as_ref()
            .unwrap()
            .video_history
            .outcome(
                "s",
                "s",
                "call",
                json!({"planned_frames":1,"image_count":0,"source":"clip.mp4"}),
            )
            .unwrap();
        super::apply(&f.core.shared, &mut row);
        assert!(row.get("vwVideo").is_none());
        f.finish().await;
    }
    #[cfg(any(windows, target_os = "macos", feature = "linux-video"))]
    #[tokio::test]
    async fn preflight_reaches_the_window_stream_before_execution_without_an_ask() {
        let f = Fixture::new().await;
        let folder = f.root.join("project");
        f.load("s", &folder, &[], true);
        let args = json!({"path":"clip.mp4","selection":{"kind":"scene_changes","start_ms":1000,"end_ms":4000,"step_ms":250,"max_frames":6},"output":"contact_sheet"});
        let state = json!({"session":{"id":"s","cwd":folder,"status":{"type":"running"}},"history":[{"id":"call","type":"effect","sessionId":"s","turnId":"turn","detail":{"toolName":"mcp_vw.video_frames","input":args},"state":{"status":"running","approvalSource":"bypass"}}],"turns":[]});
        let _ = f
            .core
            .shared
            .store
            .load(state, vw_session::LoadKind::Replace)
            .unwrap();
        f.core.shared.access.observe_request(
            "session/agent/update",
            "s",
            &json!({"agentName":"auto-approve"}),
        );
        let mut events = f.core.shared.events.subscribe();
        let future = crate::agent_tools::prepare(
            &f.core.shared,
            "s",
            "s",
            "call",
            "video_frames",
            &mut args.clone(),
        )
        .await
        .unwrap()
        .unwrap();
        let event = events.recv().await.unwrap();
        let crate::CoreEvent::Snapshot { snapshot } = event else {
            panic!("expected preflight snapshot")
        };
        assert_eq!(snapshot.state["history"][0]["vwVideo"]["plannedFrames"], 6);
        assert_eq!(snapshot.state["history"][0]["vwVideo"]["imageCount"], 1);
        assert!(snapshot.state["history"][0]["state"]["output"].is_null());
        assert_eq!(f.core.asks().len(), 0);
        drop(future); // The decoder has not run or delivered any image at this point.
        f.finish().await;
    }
}
