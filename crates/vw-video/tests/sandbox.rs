//! Real sandbox acceptance on each OS. Only explicit Linux denial runs count refusal as success.
#![cfg(feature = "helper")]
use serde_json::json;
#[tokio::test]
#[ignore = "requires verified corpus and working native isolation; Linux denial expectation is explicit"]
async fn extraction_requires_the_real_sandbox() {
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus");
    let path = std::path::Path::new(&root)
        .join("h264-baseline.mp4")
        .canonicalize()
        .unwrap();
    let input = vw_video::input::Input::copy(std::fs::File::open(&path).unwrap(), &path).unwrap();
    let job=vw_video::protocol::Job{id:"sandbox-job".into(),request:serde_json::from_value(json!({"path":path,"selection":{"kind":"timestamps","times_ms":[999]},"output":"frame"})).unwrap()};
    let helper = std::env::var_os("VW_VIDEO_HELPER").map_or_else(
        || env!("CARGO_BIN_EXE_vw-video-helper").into(),
        std::path::PathBuf::from,
    );
    let result = vw_video::sandbox::extract(&helper, input, job).await;
    if std::env::var_os("VW_VIDEO_EXPECT_SANDBOX_DENIED").is_some() {
        assert_eq!(
            std::env::consts::OS,
            "linux",
            "denial expectation is only for constrained Linux CI"
        );
        let error = result.unwrap_err();
        assert_eq!(error.code, vw_video_types::Code::WorkerFailed);
        assert!(
            error.detail.contains("namespace") || error.detail.contains("Operation not permitted"),
            "{error}"
        );
    } else {
        let reply = result.unwrap();
        assert_eq!(reply.frames[0].actual_ns, 960_000_000);
        assert_eq!(reply.image_count, 1);
    }
}

// Windows Job limits (1 GiB and 35 s of CPU time per available CPU) with Media Foundation software decoding.
// The parent still enforces a 35 s wall deadline. A dense scene scan once
// seeked back to the key frame for every sample, decoding each 1080p frame dozens of times until Windows ended
// the worker; the parent then reported an indistinct "video helper terminated (exit code: 1)".
#[cfg(windows)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test setup on the verified corpus"
)]
async fn sandboxed(
    file: &str,
    selection: serde_json::Value,
) -> Result<vw_video::protocol::Reply, vw_video_types::Error> {
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus");
    let path = std::path::Path::new(&root)
        .join(file)
        .canonicalize()
        .unwrap();
    let input = vw_video::input::Input::copy(std::fs::File::open(&path).unwrap(), &path).unwrap();
    let job = vw_video::protocol::Job {
        id: "sandbox-scan".into(),
        request: serde_json::from_value(
            json!({"path":path,"selection":selection,"output":"contact_sheet","max_edge":960}),
        )
        .unwrap(),
    };
    let helper = std::env::var_os("VW_VIDEO_HELPER").map_or_else(
        || env!("CARGO_BIN_EXE_vw-video-helper").into(),
        std::path::PathBuf::from,
    );
    vw_video::sandbox::extract(&helper, input, job).await
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires verified corpus and working native isolation"]
async fn dense_1080p_scene_scan_fits_the_worker_limits() {
    // 1920x1080, 25 fps, a key frame every 4 s: 60 samples 250 ms apart.
    let selection =
        json!({"kind":"scene_changes","start_ms":0,"end_ms":15_000,"step_ms":250,"max_frames":24});
    let reply = sandboxed("h264-1080p60s.mp4", selection).await.unwrap();
    assert_eq!(reply.scan.unwrap().samples, 60);
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires verified corpus and working native isolation"]
async fn oversized_extraction_is_a_named_timeout_not_a_worker_failure() {
    // Every request lies 3.96 s after a key frame and far from the previous one, so each one seeks and decodes
    // about 100 frames. Use 4K now that 1080p may fit the larger aggregate CPU budget; 24 x 100 4K frames
    // must still hit the unchanged wall deadline on the native acceptance computer.
    let late = (0..5).map(|k| k * 4000 + 3960);
    let order: Vec<u64> = late
        .clone()
        .zip(late.rev())
        .flat_map(|(a, b)| [a, b])
        .cycle()
        .take(24)
        .collect();
    let selection = json!({"kind":"timestamps","times_ms":order});
    let reason = sandboxed("h264-4k20s.mp4", selection).await.unwrap_err();
    assert_eq!(reason.code, vw_video_types::Code::Timeout);
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires verified corpus and working native isolation"]
async fn uhd_frame_decodes_or_names_the_memory_limit() {
    // Keep this native runner's selector, but a named memory refusal no longer passes: 1 GiB must decode 4K.
    let selection = json!({"kind":"timestamps","times_ms":[999]});
    let reply = sandboxed("h264-4k20s.mp4", selection).await.unwrap();
    assert_eq!(reply.frames.len(), 1);
    assert_eq!(reply.frames[0].actual_ns, 960_000_000);
    assert_eq!(reply.image_count, 1);
}

// Timing note: the 12-frame 4K sheet takes about 32 s of the 35 s budget with a release helper on the native
// acceptance machine; a debug helper exceeds it. Run with VW_VIDEO_HELPER set to a release build, as the native
// video runner does.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires verified corpus and working native isolation"]
async fn uhd_uniform_sheet_and_scene_scan_fit_the_worker_limits() {
    let uniform = json!({"kind":"uniform","start_ms":0,"end_ms":20_000,"count":12});
    let reply = sandboxed("h264-4k20s.mp4", uniform).await.unwrap();
    assert_eq!(reply.frames.len(), 12);
    assert_eq!(reply.image_count, 1);
    let scenes =
        json!({"kind":"scene_changes","start_ms":0,"end_ms":20_000,"step_ms":250,"max_frames":24});
    let reply = sandboxed("h264-4k20s.mp4", scenes).await.unwrap();
    assert_eq!(reply.scan.unwrap().samples, 80);
    // h264-4k20s.mp4 is a static test pattern without cuts: the full scan finishes and finds no change, so no image.
    assert_eq!(reply.image_count, 0);
}
