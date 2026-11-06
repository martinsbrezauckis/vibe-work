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
// These failures say nothing about decoder support; their next step depends on the failed check.
fn recovery(lower: &str) -> Option<(&'static str, &'static str)> {
    if lower.contains("selected model") {
        Some((
            "This model has no confirmed image support.",
            "Use metadata output, or ask the person to choose an image-capable model for frames.",
        ))
    } else if lower.contains("video jobs are active") || lower.contains("already has a video job") {
        Some((
            "Video extraction is busy.",
            "Wait for an active video job to finish before requesting another.",
        ))
    } else if lower.contains("512 mib")
        || lower.contains("source bytes")
        || lower.contains("video file size")
    {
        Some((
            "The video file is outside the input limits.",
            "Ask for a nonempty, smaller video file of at most 512 MiB.",
        ))
    } else if lower == "limit_exceeded: duration" || lower == "limit_exceeded: video duration" {
        Some((
            "The video duration is outside the supported range.",
            "Ask for a nonempty video no longer than two hours.",
        ))
    } else if lower == "limit_exceeded: video worker memory" {
        Some((
            "This video's resolution is too large for this computer's video worker.",
            "Ask for a lower-resolution copy of the video or still images.",
        ))
    } else if lower.starts_with("timeout:") {
        Some((
            "Video extraction timed out.",
            "Choose a shorter range or fewer samples, or ask for still images.",
        ))
    } else if lower.starts_with("changed_input:") {
        Some((
            "The video file changed or could not be read consistently.",
            "Use a readable local copy after it has finished changing, then request frames from that file.",
        ))
    } else if lower.starts_with("cancelled:") {
        Some((
            "Video extraction ended before completion.",
            "Continue only after the person's next message if they still want video frames.",
        ))
    } else {
        None
    }
}
/// Diagnose the native refusal for the row; retain the exact reason and a next action for the agent.
pub(crate) fn failure(reason: &str) -> (String, String) {
    let lower = reason.to_ascii_lowercase();
    if let Some((row, instruction)) = recovery(&lower) {
        return (
            row.into(),
            format!("{reason}\n{row} No video images were sent. {instruction}"),
        );
    }
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
    if lower.contains("image allowance") {
        let row = "This message's video image allowance is used.".to_owned();
        let agent = format!(
            "{reason}\n{row} No more video images were sent in this turn. Use the images already returned; more can be taken after the person's next message. Do not retry in this turn and do not claim to have seen frames that were not returned."
        );
        return (row, agent);
    }
    // Isolation, launch or supervision failures say nothing about the file or the computer's decoders; the
    // agent must not turn them into "this computer cannot decode" (a sandbox fault once read exactly like that).
    // Time/memory limits stay with their own outcomes below.
    let limit = lower.starts_with("limit_exceeded") || lower.starts_with("timeout");
    if lower.contains("worker_failed")
        || (!limit && (lower.contains("sandbox") || lower.contains("helper")))
    {
        let row = "The video worker failed.".to_owned();
        let agent = format!(
            "{reason}\n{row} Vibe Work's isolated video worker did not run correctly, so nothing is known about this video's format or this computer's decoders. No video images were sent. Tell the person the video worker failed and give the reason above; they can send still images if they need the content now. Do not say the video or this computer cannot be decoded, do not install decoders, and do not retry the same extraction."
        );
        return (row, agent);
    }
    if limit {
        let row = "Video extraction exceeded a limit.".to_owned();
        let agent = format!(
            "{reason}\n{row} No video images were sent. Use a smaller video or reduce the selected range, frame count or max_edge."
        );
        return (row, agent);
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
    let row = if let Some(codec) = codec.filter(|_| lower.contains("unsupported_codec")) {
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
    } else if lower.contains("unsupported_codec") {
        "Cannot decode this video on this computer.".into()
    } else {
        "The video frames could not be read.".into()
    };
    let agent = format!(
        "{reason}\n{row} No video images were sent. Ask for still images or a copy in a format this computer can decode, or continue using the available file metadata. Do not claim to have seen the video, install decoders, or retry the same unsupported extraction."
    );
    (row, agent)
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
    #[test]
    fn non_decoder_failures_give_the_relevant_next_step() {
        for (reason, instruction) in [
            (
                "unsupported_codec: the selected model has no confirmed image support",
                "image-capable model",
            ),
            (
                "limit_exceeded: two video jobs are active",
                "Wait for an active video job",
            ),
            (
                "limit_exceeded: regular video must be between 1 byte and 512 MiB",
                "smaller video file",
            ),
            ("timeout: video job deadline", "shorter range"),
            ("limit_exceeded: duration", "no longer than two hours"),
            ("limit_exceeded: video duration", "no longer than two hours"),
            ("limit_exceeded: helper resident memory", "smaller video"),
            (
                "changed_input: source changed during snapshot",
                "finished changing",
            ),
            ("cancelled: video job ended", "next message"),
        ] {
            let (_, text) = failure(reason);
            assert!(text.contains(reason), "{text}");
            assert!(text.contains(instruction), "{text}");
            assert!(!text.contains("format this computer can decode"), "{text}");
        }
    }
    #[test]
    fn worker_memory_limit_names_the_resolution_not_missing_decoding() {
        let reason = "limit_exceeded: video worker memory";
        let (row, agent) = failure(reason);
        assert_eq!(
            row,
            "This video's resolution is too large for this computer's video worker."
        );
        assert!(agent.starts_with(reason), "{agent}");
        assert!(agent.contains("lower-resolution copy"), "{agent}");
        assert!(agent.contains("still images"), "{agent}");
        assert!(!agent.contains("decode"), "{agent}");
    }
    use serde_json::json;
    #[test]
    fn native_unsupported_media_is_named_as_cannot_decode_with_reason_and_instruction() {
        for reason in [
            "unsupported_codec: Media Foundation decoder or media support unavailable (0xc00d5212)",
            "unsupported_codec: AVFoundation could not decode avc1",
        ] {
            let (row, agent) = failure(reason);
            assert!(row.starts_with("Cannot decode this video"), "{row}");
            assert!(agent.contains(reason));
            assert!(agent.contains("Ask for still images"));
            assert!(agent.contains("Do not claim to have seen"));
        }
    }
    #[test]
    fn worker_or_sandbox_failure_is_named_as_such_not_as_missing_decoding() {
        for reason in [
            "worker_failed: AVFoundation could not open the video snapshot",
            "backend_unavailable: macOS media sandbox denied",
            "backend_unavailable: helper requires its sandbox",
            "backend_unavailable: Windows media sandbox denied",
            "backend_unavailable: Linux sandbox namespaces or media access denied",
            "backend_unavailable: helper sandbox could not start",
            "backend_unavailable: video helper is missing",
            "decode_failed: video helper terminated (signal: 25 (SIGXFSZ))",
            "decode_failed: helper reply shape",
        ] {
            let (row, agent) = failure(reason);
            assert_eq!(row, "The video worker failed.", "{reason}");
            assert!(agent.starts_with(reason), "{agent}");
            assert!(
                agent.contains("Vibe Work's isolated video worker"),
                "{agent}"
            );
            assert!(agent.contains("Tell the person"), "{agent}");
            assert!(agent.contains("Do not say"), "{agent}");
            assert!(!agent.contains("Cannot decode"), "{agent}");
            assert!(
                !agent.contains("format this computer can decode"),
                "{agent}"
            );
        }
    }
    #[test]
    fn used_turn_image_allowance_is_not_called_a_decoding_problem() {
        let reason = "limit_exceeded: this user turn's image allowance is used";
        let (row, agent) = failure(reason);
        assert_eq!(row, "This message's video image allowance is used.");
        assert!(agent.starts_with(reason), "{agent}");
        assert!(agent.contains("next message"), "{agent}");
        assert!(!agent.contains("decode"), "{agent}");
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
    #[cfg(target_os = "linux")]
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
    #[cfg(any(windows, target_os = "macos"))]
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
