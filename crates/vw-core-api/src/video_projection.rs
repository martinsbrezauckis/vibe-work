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
/// Diagnose the native refusal for the row; retain the exact reason and a next action for the agent.
pub(crate) fn failure(reason: &str) -> (String, String) {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("range_outside_video")
        || lower.contains("limit_exceeded: select")
        || lower.contains("limit_exceeded: scene scanning")
    {
        return (
            "The selected video range is unavailable.".into(),
            format!(
                "{reason} No video images were sent. Choose a valid range or sampling step and request the selection again; do not claim to have seen the refused range."
            ),
        );
    }
    let codec = [
        ("h.264", "H.264"),
        ("h264", "H.264"),
        ("hevc", "HEVC"),
        ("h265", "HEVC"),
        ("av1", "AV1"),
        ("vp9", "VP9"),
        ("vp8", "VP8"),
    ]
    .iter()
    .find(|(needle, _)| lower.contains(needle))
    .map(|(_, name)| *name);
    let row = if lower.contains("selected model") {
        "This model cannot read images.".to_owned()
    } else if let Some(codec) = codec.filter(|_| lower.contains("unsupported_codec")) {
        format!("Cannot decode this video: {codec} support is missing.")
    } else if lower.contains("qtdemux") {
        "Cannot decode this video: MP4 support is missing.".into()
    } else if lower.contains("matroskademux") {
        "Cannot decode this video: WebM support is missing.".into()
    } else if lower.contains("backend_unavailable")
        && (lower.contains("gstreamer") || lower.contains("appsink"))
    {
        "Cannot decode this video: GStreamer media support is missing.".into()
    } else if lower.contains("unsupported_container") {
        "Cannot decode this video format.".into()
    } else if lower.contains("sandbox") {
        "Cannot decode this video: video isolation is unavailable.".into()
    } else {
        "The video frames could not be read.".into()
    };
    let agent = format!(
        "{reason}\n{row} No video images were sent. Ask for still images or a copy in a format this computer can decode, or continue using the available file metadata. Do not claim to have seen the video, install decoders, or retry the same unsupported extraction."
    );
    (row, agent)
}
/// Called at all session snapshot/event boundaries; absent from default builds.
pub(crate) fn apply(shared: &crate::core::Shared, value: &mut Value) {
    project(value);
    if let Some(tools) = &shared.agent_tools {
        tools.video_history.overlay(value);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    #[test]
    fn missing_support_names_codec_and_tells_agent_what_to_do_next() {
        let (row, agent) = failure(
            "unsupported_codec: H.264; caps video/x-h264; missing installed element avdec_h264",
        );
        assert_eq!(row, "Cannot decode this video: H.264 support is missing.");
        assert!(agent.contains("H.264"));
        assert!(agent.contains("Ask for"));
        assert!(agent.contains("Do not"));
    }
}

#[cfg(test)]
mod delivery_tests {
    use crate::feature_test_core::Fixture;
    use serde_json::json;
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
