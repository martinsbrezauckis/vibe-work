//! Owned fixture acceptance through the helper's real extraction code and the MCP stdio image boundary.
//! Tests are explicit opt-ins: no corpus, codec, sandbox or selected-model support is inferred from a skip.
#![cfg(feature = "helper")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use serde_json::{Value, json};
fn job(file: &str, times: &[u64], output: &str) -> vw_video::protocol::Job {
    let root = std::env::var_os("VW_VIDEO_CORPUS").expect("verified VW_VIDEO_CORPUS");
    vw_video::protocol::Job{id:"owned-job".into(),request:serde_json::from_value(json!({"path":std::path::Path::new(&root).join(file),"selection":{"kind":"timestamps","times_ms":times},"output":output})).unwrap()}
}
#[test]
#[ignore = "requires the owned corpus and the video MCP feature"]
fn known_video_frame_is_real_mcp_image_content() {
    let j = job("h264-baseline.mp4", &[999], "frame");
    let reply = vw_video::worker::extract(&j).unwrap();
    vw_video::protocol::validate(&j, &reply).unwrap();
    assert_eq!(reply.frames[0].actual_ns, 960_000_000);
    let expected = reply.png.clone();
    let mut result = Some(reply.into_tool());
    let input =
        json!({"id":1,"method":"tools/call","params":{"name":"video_frames","arguments":j.request}})
            .to_string() + "\n";
    let mut output = Vec::new();
    vw_agent_tools::mcp::serve(input.as_bytes(), &mut output, |name: &str, _: Value| {
        assert_eq!(name, "video_frames");
        Ok(result.take().unwrap())
    })
    .unwrap();
    let messages: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let result = &messages.last().unwrap()["result"];
    assert_eq!(result["content"][0]["type"], "image");
    assert_eq!(result["content"][0]["data"], expected);
    assert_eq!(result["content"][0]["mimeType"], "image/png");
    assert!(result["structuredContent"].get("vwImage").is_none());
    assert!(result["structuredContent"].get("path").is_none());
}
#[test]
#[ignore = "requires the owned corpus and installed system video decoders"]
fn sheets_have_bounded_geometry_bytes_and_all_actual_timestamps() {
    for file in [
        "h264-baseline.mp4",
        "h264-vfr.mp4",
        "h264-rot90.mp4",
        "h264-par.mp4",
        "vp8-cues.webm",
        "vp9-no-cues.webm",
        "vp9-odd.webm",
        "h264-fragmented.mp4",
        "h264-aac.mp4",
    ] {
        let j = job(file, &[41, 999, 2501, 3999], "contact_sheet");
        let reply = match vw_video::worker::extract(&j) {
            Ok(reply) => reply,
            Err(reason)
                if !cfg!(target_os = "linux")
                    && std::path::Path::new(file)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
                    && (reason.starts_with("unsupported_codec:")
                        || reason.starts_with("unsupported_container:")) =>
            {
                eprintln!("NOT RUN: {file} sheet pixels; this computer cannot decode it: {reason}");
                continue;
            }
            Err(reason) => panic!("{file}: {reason}"),
        };
        vw_video::protocol::validate(&j, &reply).unwrap();
        assert_eq!(reply.frames.len(), 4);
        assert_eq!(reply.image_count, 1);
        assert!(reply.image_size.iter().all(|n| *n <= vw_preview::MAX_WIDTH));
        assert!(reply.png.len() <= vw_preview::MAX_BYTES.div_ceil(3) * 4);
    }
}
#[test]
#[ignore = "requires the owned corpus and installed system video decoders"]
fn incomplete_container_never_becomes_a_ready_sheet() {
    for file in ["malformed-mp4.mp4", "truncated-mp4.mp4"] {
        assert!(
            vw_video::worker::extract(&job(file, &[0], "contact_sheet")).is_err(),
            "{file}"
        );
    }
}

#[test]
#[ignore = "requires all owned fixtures and the host's installed decoders"]
fn complete_owned_corpus_has_frames_or_named_unsupported_support() {
    for file in [
        "h264-baseline.mp4",
        "h264-main-bframes.mp4",
        "h264-high-long.mp4",
        "h264-short-gop.mp4",
        "h264-vfr.mp4",
        "h264-rot90.mp4",
        "h264-par.mp4",
        "h264-high10.mp4",
        "h264-fragmented.mp4",
        "h264-aac.mp4",
        "hevc-main.mp4",
        "hevc-main10.mp4",
        "vp8-cues.webm",
        "vp8-no-cues.webm",
        "vp9-cues.webm",
        "vp9-no-cues.webm",
        "vp9-10bit.webm",
        "vp9-odd.webm",
        "av1-mp4.mp4",
        "av1-webm.webm",
        "h264-1080p60s.mp4",
        "h264-4k20s.mp4",
    ] {
        let j = job(file, &[999], "frame");
        match vw_video::worker::extract(&j) {
            Ok(reply) => {
                vw_video::protocol::validate(&j, &reply).unwrap();
                eprintln!("{file}: ready, PTS {} ns", reply.frames[0].actual_ns);
            }
            Err(reason) => {
                assert!(
                    reason.starts_with("unsupported_codec:")
                        || reason.starts_with("unsupported_container:")
                        || reason.starts_with("backend_unavailable:"),
                    "{file}: {reason}"
                );
                eprintln!("{file}: {reason}");
            }
        }
    }
    incomplete_container_never_becomes_a_ready_sheet();
}

#[test]
#[ignore = "requires owned 4K fixture and installed system decoder; applies hard limits in a fresh child"]
#[cfg(target_os = "linux")]
fn overview_under_helper_memory_limit() {
    let plugins = tempfile::tempdir().unwrap();
    for path in vw_video::sandbox::system_plugins() {
        std::os::unix::fs::symlink(&path, plugins.path().join(path.file_name().unwrap())).unwrap();
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "overview_limited_child",
            "--ignored",
            "--nocapture",
        ])
        .env("VW_VIDEO_OVERVIEW_CHILD", "1")
        .env("MALLOC_ARENA_MAX", "1")
        .env("ORC_CODE", "backup")
        .env("GST_PLUGIN_SYSTEM_PATH_1_0", plugins.path())
        .env("GST_REGISTRY_FORK", "no")
        .env("GST_PLUGIN_PATH_1_0", "")
        .env("GST_REGISTRY_1_0", "/nonexistent/registry")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let errors =
        std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::stderr()).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    errors.join().unwrap();
    eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "status={}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "invoked only by the bounded overview parent"]
#[cfg(target_os = "linux")]
fn overview_limited_child() {
    if std::env::var_os("VW_VIDEO_OVERVIEW_CHILD").is_none() {
        return;
    }
    let mut j = job("h264-4k20s.mp4", &[1], "contact_sheet");
    j.request.selection = vw_agent_tools::video::Selection::Uniform {
        start_ms: 0,
        end_ms: 20_000,
        count: 12,
    };
    vw_platform::video::limits::restrict().unwrap();
    let started = std::time::Instant::now();
    let reply = vw_video::worker::extract(&j).unwrap();
    vw_video::protocol::validate(&j, &reply).unwrap();
    assert_eq!(reply.frames.len(), 12);
    eprintln!(
        "4K overview: {} ms, image {}x{}, {} base64 bytes",
        started.elapsed().as_millis(),
        reply.image_size[0],
        reply.image_size[1],
        reply.png.len()
    );
    j.request.selection = vw_agent_tools::video::Selection::SceneChanges {
        start_ms: 0,
        end_ms: 20_000,
        step_ms: 1000,
        max_frames: 24,
    };
    let started = std::time::Instant::now();
    let reply = vw_video::worker::extract(&j).unwrap();
    vw_video::protocol::validate(&j, &reply).unwrap();
    assert_eq!(reply.scan.as_ref().unwrap().samples, 20);
    assert!(reply.frames.len() <= 24);
    eprintln!(
        "4K scene scan: {} ms, {} selected frames, 20 samples",
        started.elapsed().as_millis(),
        reply.frames.len()
    );
}
