//! Per-format packaged-helper check: one frame from every video in a folder through the real sandbox
//! launcher, exactly as the app calls it. Point `VW_VIDEO_HELPER` at an installed or extracted package helper (for
//! an `AppImage`: `squashfs-root/usr/bin/vw-video-helper`, which selects its launcher and the system media runtime).
//! `VW_VIDEO_FORMATS` names the folder; `VW_VIDEO_FORMATS_EXPECT` optionally pins each outcome as
//! `file=ok` or `file=<error code>` entries separated by `;`, and the test fails on any difference.
#![cfg(feature = "helper")]
use serde_json::json;
#[tokio::test]
#[ignore = "requires a packaged helper, a video folder and working native isolation"]
async fn every_video_in_the_folder_reports_its_outcome() {
    let root =
        std::path::PathBuf::from(std::env::var_os("VW_VIDEO_FORMATS").expect("video folder"));
    let helper = std::env::var_os("VW_VIDEO_HELPER").map_or_else(
        || env!("CARGO_BIN_EXE_vw-video-helper").into(),
        std::path::PathBuf::from,
    );
    let expect: Vec<(String, String)> = std::env::var("VW_VIDEO_FORMATS_EXPECT")
        .unwrap_or_default()
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.split_once('=').expect("expected file=outcome"))
        .map(|(f, o)| (f.trim().to_owned(), o.trim().to_owned()))
        .collect();
    let mut names: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "format folder is empty");
    for (name, outcome) in &expect {
        assert!(!outcome.is_empty(), "empty expected outcome for {name}");
        assert!(
            names
                .iter()
                .any(|p| p.file_name().unwrap() == name.as_str()),
            "missing expected fixture: {name}"
        );
    }
    let mut mismatches = Vec::new();
    for path in names {
        let path = path.canonicalize().unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let input =
            vw_video::input::Input::copy(std::fs::File::open(&path).unwrap(), &path).unwrap();
        let job = vw_video::protocol::Job {
            id: format!("formats-{name}"),
            request: serde_json::from_value(json!({"path": path,
                "selection": {"kind": "timestamps", "times_ms": [1000]}, "output": "frame"}))
            .unwrap(),
        };
        let outcome = match vw_video::sandbox::extract(&helper, input, job).await {
            Ok(reply) => {
                println!(
                    "FORMAT | {name} | OK | codec={} size={}x{} frame_ns={} images={} backend={}",
                    reply.codec,
                    reply.source_size[0],
                    reply.source_size[1],
                    reply.frames[0].actual_ns,
                    reply.image_count,
                    reply.backend
                );
                "ok".to_owned()
            }
            Err(error) => {
                println!("FORMAT | {name} | ERR | {error}");
                serde_json::to_value(error.code).unwrap()["code"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
        };
        if let Some((_, wanted)) = expect.iter().find(|(f, _)| *f == name) {
            let matched = if wanted == "ok" {
                outcome == "ok"
            } else {
                outcome == *wanted
            };
            if !matched {
                mismatches.push(format!("{name}: expected {wanted}, got {outcome}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}
