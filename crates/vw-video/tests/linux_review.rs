//! Linux review acceptance for generated codec fixtures: actual sandboxed sheets, out-of-order seeks and cuts.
//! The native runner supplies `linux_formats.py` output and, optionally, an installed package helper.
#![cfg(all(target_os = "linux", feature = "helper"))]
use serde_json::{Value, json};
use std::path::Path;

async fn review(path: &Path, selection: Value) -> Result<vw_video::protocol::Reply, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let source = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let input = vw_video::input::Input::copy(source, &path).map_err(|e| e.to_string())?;
    let helper = std::env::var_os("VW_VIDEO_HELPER").map_or_else(
        || env!("CARGO_BIN_EXE_vw-video-helper").into(),
        std::path::PathBuf::from,
    );
    let job = vw_video::protocol::Job {
        id: "review".into(),
        request: serde_json::from_value(json!({"path":path,"selection":selection,
            "output":"contact_sheet","max_edge":960}))
        .map_err(|error| error.to_string())?,
    };
    vw_video::sandbox::extract(&helper, input, job)
        .await
        .map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore = "requires linux_formats.py fixtures and working Linux isolation"]
async fn every_codec_delivers_a_sheet_with_correct_out_of_order_presentation_times() {
    let root =
        std::path::PathBuf::from(std::env::var_os("VW_VIDEO_FORMATS").expect("Linux formats"));
    for name in [
        "h264-baseline.mp4",
        "h264-bframes.mp4",
        "h264-4k-refs.mp4",
        "h265-hvc1.mp4",
        "h265-hev1.mp4",
        "h265-1080p.mp4",
        "vp8.webm",
        "vp9.webm",
        "av1.mp4",
        "av1.webm",
    ] {
        let reply = review(
            &root.join(name),
            json!({"kind":"timestamps","times_ms":[3999,41,2501,999]}),
        )
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(reply.image_count, 1, "{name}");
        assert_eq!(
            reply.frames.iter().map(|f| f.actual_ns).collect::<Vec<_>>(),
            [3_960_000_000, 40_000_000, 2_480_000_000, 960_000_000],
            "{name}"
        );
        println!("SHEET | {name} | OK | {}", reply.backend);
    }
}

#[tokio::test]
#[ignore = "requires the committed scene fixture and working Linux isolation"]
async fn scene_changes_reach_the_real_sandbox_with_exact_cut_times() {
    let root = std::path::PathBuf::from(
        std::env::var_os("VW_VIDEO_CORPUS").expect("scene fixture folder"),
    );
    let path = root.join("scene-cuts.mp4");
    let reply = review(
        &path,
        json!({"kind":"scene_changes","start_ms":0,"end_ms":4000,"step_ms":250,"max_frames":24}),
    )
    .await
    .unwrap();
    assert_eq!(reply.scan.unwrap().samples, 16);
    assert_eq!(reply.image_count, 1);
    assert_eq!(
        reply.frames.iter().map(|f| f.actual_ns).collect::<Vec<_>>(),
        [1_000_000_000, 2_000_000_000, 3_000_000_000]
    );
}
