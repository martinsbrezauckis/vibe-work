//! Real Linux sandbox acceptance. The denial test is runnable in restricted CI; success requires namespaces.
#![cfg(all(feature = "helper", target_os = "linux"))]
use serde_json::json;
#[tokio::test]
#[ignore = "requires verified corpus and Linux namespaces; denial expectation is explicit"]
async fn extraction_requires_the_real_sandbox() {
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus");
    let path = std::path::Path::new(&root)
        .join("h264-baseline.mp4")
        .canonicalize()
        .unwrap();
    let input = vw_video::input::Input::copy(std::fs::File::open(&path).unwrap(), &path).unwrap();
    let job=vw_video::protocol::Job{id:"sandbox-job".into(),request:serde_json::from_value(json!({"path":path,"selection":{"kind":"timestamps","times_ms":[999]},"output":"frame"})).unwrap()};
    let result = vw_video::sandbox::extract(
        std::path::Path::new(env!("CARGO_BIN_EXE_vw-video-helper")),
        input,
        job,
    )
    .await;
    if std::env::var_os("VW_VIDEO_EXPECT_SANDBOX_DENIED").is_some() {
        assert!(
            result
                .unwrap_err()
                .contains("backend_unavailable: Linux sandbox")
        );
    } else {
        let reply = result.unwrap();
        assert_eq!(reply.frames[0].actual_ns, 960_000_000);
        assert_eq!(reply.image_count, 1);
    }
}
