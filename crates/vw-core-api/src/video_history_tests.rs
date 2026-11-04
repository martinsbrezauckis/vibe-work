//! Regression tests for saved video image ownership, byte limits, restart and content-version checks.
use super::*;
use serde_json::json;

fn folder() -> PathBuf {
    let root = std::env::var_os("VW_TEST_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    let path = root.join(vw_agent_tools::credential::new_credential().unwrap());
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn reply(id: &str, size: usize) -> Value {
    json!({"status":"ready","video_id":id,"source":"clip.mp4", "planned_frames":3,"image_count":1,
      "image_size":[1200,240],"columns":3,"source_size":[640,360],"rotation_applied":0,"pixel_aspect_ratio":[1,1],
      "frames":[{"index":0,"frame_id":0,"actual_ns":920_000_000,"requested_ms":999},{"index":1,"frame_id":0,"actual_ns":920_000_000,"requested_ms":1000},{"index":2,"frame_id":1,"actual_ns":2_480_000_000_u64,"requested_ms":2501}],
      vw_agent_tools::wire::IMAGE_KEY:{"mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(vec![42;size])}})
}
#[test]
fn saved_cells_survive_restart_without_source_and_are_session_owned() {
    let cwd = folder();
    let index = cwd.join("index.json");
    let history = History::load(index.clone()).unwrap();
    history
        .save(&cwd, "chat", "chat", "call", reply("job", 100))
        .unwrap();
    drop(history);
    let history = History::load(index).unwrap();
    let content = history.read("chat", "video:job:1").unwrap();
    assert!(
        content
            .data_url
            .unwrap()
            .starts_with("data:image/svg+xml;base64,")
    );
    assert!(history.read("other", "video:job:1").is_err());
    assert!(history.read("chat", "video:job:2").is_err());
    history.forget("chat").unwrap();
    assert!(history.read("chat", "video:job:1").is_err());
    assert_eq!(
        std::fs::read_dir(cwd.join("attachments/chat"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        history
            .save(&cwd, "chat", "chat", "late", reply("late", 100))
            .is_err()
    );
    std::fs::remove_dir_all(cwd).unwrap();
}
#[test]
fn changed_bytes_and_links_never_open_as_the_returned_frame() {
    let cwd = folder();
    let h = History::load(cwd.join("index.json")).unwrap();
    h.save(&cwd, "chat", "chat", "call", reply("job", 100))
        .unwrap();
    let file = std::fs::read_dir(cwd.join("attachments/chat"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(&file, vec![43; 100]).unwrap();
    assert!(
        h.read("chat", "video:job:0")
            .unwrap_err()
            .contains("changed")
    );
    #[cfg(unix)]
    {
        std::fs::remove_file(&file).unwrap();
        std::fs::write(cwd.join("outside"), vec![42; 100]).unwrap();
        std::os::unix::fs::symlink(cwd.join("outside"), &file).unwrap();
        assert!(h.read("chat", "video:job:0").is_err());
    }
    std::fs::remove_dir_all(cwd).unwrap();
}
#[test]
fn session_byte_cap_evicts_oldest_and_single_image_cap_refuses() {
    let cwd = folder();
    let h = History::load(cwd.join("index.json")).unwrap();
    assert!(
        h.save(&cwd, "chat", "chat", "huge", reply("huge", 1024 * 1024 + 1))
            .is_err()
    );
    for i in 0..33 {
        h.save(
            &cwd,
            "chat",
            "chat",
            &format!("call-{i}"),
            reply(&format!("job-{i}"), 1024 * 1024),
        )
        .unwrap();
    }
    assert!(h.read("chat", "video:job-0:0").is_err());
    assert!(h.read("chat", "video:job-32:0").is_ok());
    assert_eq!(
        std::fs::read_dir(cwd.join("attachments/chat"))
            .unwrap()
            .count(),
        32
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
#[cfg(feature = "video-test")]
#[ignore = "requires the verified owned corpus and vw-video/helper"]
fn owned_frames_feed_saved_cell_viewer() {
    // Compiled only when the helper is enabled in the acceptance command below.
    let corpus = PathBuf::from(std::env::var_os("VW_VIDEO_CORPUS").expect("verified corpus"));
    let cwd = folder();
    let h = History::load(cwd.join("index.json")).unwrap();
    let mut fixtures = serde_json::Map::new();
    let mut cases: Vec<(&str, usize, &str, Value)> =
        [("1", 1), ("6", 6), ("24", 24), ("sheet", 12)]
            .into_iter()
            .map(|(scene, count)| {
                (
                    scene,
                    count,
                    "h264-vfr.mp4",
                    json!({"kind":"uniform","start_ms":0,"end_ms":4000,"count":count}),
                )
            })
            .collect();
    for (scene, start, end, cap, count) in [
        ("scenes", 0, 4000, 24, 3),
        ("scene-range", 1250, 2750, 24, 1),
        ("scene-limit", 0, 4000, 2, 2),
        ("no-cuts", 0, 750, 24, 0),
    ] {
        cases.push((scene,count,"scene-cuts.webm",json!({"kind":"scene_changes","start_ms":start,"end_ms":end,"step_ms":250,"max_frames":cap})));
    }
    cases.push((
        "overview-range",
        2,
        "scene-cuts.webm",
        json!({"kind":"uniform","start_ms":1000,"end_ms":3000,"count":2}),
    ));
    cases.push((
        "interval-range",
        3,
        "scene-cuts.webm",
        json!({"kind":"interval","start_ms":1000,"end_ms":4000,"every_ms":1000,"max_frames":3}),
    ));
    for (scene, count, file, selection) in cases {
        let output = if scene == "1" {
            "frame"
        } else {
            "contact_sheet"
        };
        let request = serde_json::from_value(
            json!({"path":corpus.join(file),"selection":selection,"output":output}),
        )
        .unwrap();
        let job = vw_video::protocol::Job {
            id: format!("owned-{scene}"),
            request,
        };
        let reply = vw_video::worker::extract(&job).unwrap();
        vw_video::protocol::validate(&job, &reply).unwrap();
        let mut meta = reply.into_tool();
        meta["output"] = json!(output);
        meta["sheet_edge"] = json!(1280);
        meta["source"] = json!(file);
        if count > 0 {
            h.save(&cwd, "chat", "chat", scene, meta.clone()).unwrap();
        } else {
            h.outcome("chat", "chat", scene, meta.clone()).unwrap();
        }
        let mut content = serde_json::Map::new();
        for frame in meta["frames"].as_array().unwrap() {
            let handle = format!("video:owned-{scene}:{}", frame["frame_id"]);
            let image = h.read("chat", &handle).unwrap();
            let svg = base64::engine::general_purpose::STANDARD
                .decode(image.data_url.as_ref().unwrap().split_once(',').unwrap().1)
                .unwrap();
            let svg = String::from_utf8(svg).unwrap();
            assert!(svg.contains("data:image/png;base64,"));
            if scene == "scenes" {
                assert!(svg.contains("width=\"426\" height=\"319\""), "{svg:.150}");
            }
            let template = svg.replace(
                meta[vw_agent_tools::wire::IMAGE_KEY]["data"]
                    .as_str()
                    .unwrap(),
                "VIDEO_IMAGE_DATA",
            );
            let mut fields = serde_json::to_value(image).unwrap();
            fields["dataUrl"] = Value::Null;
            content.insert(handle, json!({"fields":fields,"svg":template}));
        }
        let projection = crate::video_image::projection(&meta);
        assert_eq!(projection["frames"].as_array().unwrap().len(), count);
        fixtures.insert(
            scene.into(),
            json!({"projection":projection,"png":meta[vw_agent_tools::wire::IMAGE_KEY]["data"],"content":content}),
        );
    }
    if let Some(output) = std::env::var_os("VW_VIDEO_UI_FIXTURES_OUTPUT") {
        std::fs::write(output, serde_json::to_vec(&fixtures).unwrap()).unwrap();
    }
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn deletion_forgets_missing_attachment_directories() {
    let cwd = folder();
    let index = cwd.join("index.json");
    let h = History::load(index.clone()).unwrap();
    h.save(&cwd, "chat", "chat", "call", reply("job", 100))
        .unwrap();
    std::fs::remove_dir_all(cwd.join("attachments")).unwrap();
    h.forget("chat").unwrap();
    let restarted = History::load(index).unwrap();
    assert_eq!(restarted.state.lock().unwrap().records.len(), 0);
    std::fs::remove_dir_all(cwd).unwrap();
}
