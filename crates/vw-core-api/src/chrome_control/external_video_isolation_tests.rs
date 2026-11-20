//! External video across chats and browsers: distinct default files, isolated frames and files for two chats, a
//! helper's start refusal passed through unchanged, and Safari's bare profile identity matched to its session row.
use super::external_video_adversarial_tests::{
    core, identity, request, saved_stop_result, screenshot, set_chat_folder, start_result, status,
    stopped_result,
};
use base64::Engine as _;
use serde_json::{Value, json};
use vw_browser::recording::VideoCommand;

#[tokio::test]
async fn sequential_default_filenames_save_two_distinct_webms() {
    let (core, encoder) = core("external-video-default-names").await;
    let seen = super::test_support::answering(&core.shared, |tool, args| match tool {
        "chrome_status" => status(),
        "chrome_execute" if args["action"] == "recording.video.start" => start_result(2),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            screenshot(&args["_approvedCapabilities"])
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => saved_stop_result(),
        _ => panic!("unexpected {tool}: {args}"),
    });
    let mut paths = Vec::new();
    for _ in 0..2 {
        super::read_consent::send(
            &core.shared,
            "s",
            "chrome_execute",
            request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
            None,
        )
        .await
        .unwrap();
        let stopped = super::read_consent::send(
            &core.shared,
            "s",
            "chrome_execute",
            request("recording.video.stop", json!({"tabId":7})),
            None,
        )
        .await
        .unwrap();
        let data = &stopped.value()["structuredContent"]["data"];
        assert_eq!(data["state"], "saved");
        assert_eq!(data["mimeType"], "video/webm");
        assert_eq!(data["frames"], 1);
        let path = data["file"]["path"].as_str().unwrap().to_owned();
        assert!(
            path.starts_with("browser-")
                && std::path::Path::new(&path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
        );
        let folder = super::output_file::folder(&core.shared, "s").unwrap();
        assert_eq!(folder.metadata(&path).unwrap().len(), 8);
        paths.push(path);
    }
    assert_ne!(paths[0], paths[1]);
    assert_eq!(
        encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, VideoCommand::Finish { .. }))
            .count(),
        2
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(
                |(tool, args)| tool == "chrome_execute" && args["action"] == "recording.video.stop"
            )
            .count(),
        2
    );
    core.shutdown().await;
}

fn two_chat_answer(tool: &str, args: &Value) -> Value {
    let session = args["sessionId"].as_str().unwrap_or("t");
    let mut id = identity();
    if session == "u" {
        id["recordingId"] = json!("video-other");
        id["tabId"] = json!(8);
        id["documentId"] = json!("doc-other");
    }
    match tool {
        "chrome_status" => {
            json!({"structuredContent":{"profiles":[{"profileId":"firefox:agent","browserSessionId":"browser-test","connected":true,
            "extensionCapabilities":["page.video.frames.v1"],
            "features":["recording.video.start","recording.video.stop","capture.screenshot"]}],
            "sessions":[
                {"sessionId":"t","profileId":"firefox:agent","state":"active","leaseEpoch":1,"policyRevision":1,"target":{"tabId":7,"documentId":"doc-test"}},
                {"sessionId":"u","profileId":"firefox:agent","state":"active","leaseEpoch":1,"policyRevision":1,"target":{"tabId":8,"documentId":"doc-other"}}
            ]}})
        }
        "chrome_execute" if args["action"] == "recording.video.start" => {
            id["state"] = json!("recording");
            id["appEncoder"] = json!(true);
            id["maxSeconds"] = json!(2);
            json!({"structuredContent":{"status":"completed","data":id}})
        }
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            assert!(
                !args["_approvedCapabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("auth.secrets"))
            );
            let capture = json!({"profileId":id["profileId"],"browserSessionId":id["browserSessionId"],
                    "leaseEpoch":id["leaseEpoch"],"policyRevision":id["policyRevision"],
                    "documentId":id["documentId"],"tabId":id["tabId"],"recordingId":id["recordingId"]});
            json!({"structuredContent":{"status":"completed","data":{"mimeType":"image/png","captureIdentity":capture}},
                    "content":[{"type":"image","mimeType":"image/png",
                    "data":base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nmock")}]})
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => {
            id["state"] = json!("stopped");
            id["appEncoder"] = json!(true);
            json!({"structuredContent":{"status":"completed","data":id}})
        }
        _ => panic!("unexpected {tool}: {args}"),
    }
}

// PREDICT: two chats' simultaneous recordings could mix frames, Stop receipts or saved files.
#[tokio::test]
async fn simultaneous_chats_keep_frames_and_files_isolated() {
    let (core, encoder) = core("external-video-two-chats").await;
    super::test_support::chat(&core.shared, "other");
    set_chat_folder(&core, "other");
    core.shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("u".into(), "other".into());
    core.shared.chrome_control.work.lock().unwrap().open(
        "u",
        "other",
        "active",
        "firefox:agent",
        None,
    );
    let seen = super::test_support::answering(&core.shared, two_chat_answer);
    for (chat, session, tab) in [("s", "t", 7), ("other", "u", 8)] {
        super::read_consent::send(
            &core.shared, chat, "chrome_execute",
            json!({"sessionId":session,"action":"recording.video.start","args":{"tabId":tab,"maxSeconds":2}}), None,
        ).await.unwrap();
    }
    let mut paths = Vec::new();
    for (chat, session, tab) in [("s", "t", 7), ("other", "u", 8)] {
        let result = super::read_consent::send(
            &core.shared,
            chat,
            "chrome_execute",
            json!({"sessionId":session,"action":"recording.video.stop","args":{"tabId":tab}}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            result.value()["structuredContent"]["data"]["state"],
            "saved"
        );
        paths.push(
            result.value()["structuredContent"]["data"]["file"]["path"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let own_folder = super::output_file::folder(&core.shared, "s").unwrap();
    let other_folder = super::output_file::folder(&core.shared, "other").unwrap();
    assert!(own_folder.metadata(&paths[0]).is_ok());
    assert!(own_folder.metadata(&paths[1]).is_err());
    assert!(other_folder.metadata(&paths[1]).is_ok());
    assert!(other_folder.metadata(&paths[0]).is_err());
    {
        let commands = encoder.0.lock().unwrap();
        for recording in ["video-test", "video-other"] {
            assert!(commands.iter().any(
                |c| matches!(c, VideoCommand::Frame { recording: id, .. } if id == recording)
            ));
            assert!(commands.iter().any(
                |c| matches!(c, VideoCommand::Finish { recording: id, .. } if id == recording)
            ));
        }
    }
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(
                |(tool, args)| tool == "chrome_execute" && args["action"] == "recording.video.stop"
            )
            .count(),
        2
    );
    core.shutdown().await;
}

// PREDICT: the app replaced the helper's own Start refusal with a generic text, hiding why the browser refused.
#[tokio::test]
async fn helper_start_refusal_reaches_the_agent_unchanged() {
    let (core, encoder) = core("external-video-helper-refusal").await;
    let refusal = json!({"isError":true,"structuredContent":{"status":"failed","error":{
        "code":"CAPTURE_UNAVAILABLE","message":"Safari can capture only the active tab. Focus the tab first."}}});
    let seen = super::test_support::answering(&core.shared, {
        let refusal = refusal.clone();
        move |tool, args| match tool {
            "chrome_status" => status(),
            "chrome_execute" if args["action"] == "recording.video.start" => refusal.clone(),
            "chrome_execute" if args["action"] == "recording.video.stop" => stopped_result(),
            _ => panic!("unexpected {tool}: {args}"),
        }
    });
    let answer = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .expect("a helper refusal is the browser's own receipt, not an app error");
    assert_eq!(
        answer.value()["structuredContent"]["error"]["code"],
        "CAPTURE_UNAVAILABLE"
    );
    assert!(encoder.0.lock().unwrap().is_empty());
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|(_, args)| args["action"] == "capture.screenshot")
    );
    core.shutdown().await;
}

// PREDICT (native Safari 27.2): Safari's extension stamps its bare profile UUID into the recording
// identity while the helper's session row uses `safari:<uuid>`, so every first frame failed "The browser session ended".
#[tokio::test]
async fn safari_bare_profile_identity_matches_its_namespaced_session_row() {
    let (core, encoder) = core("external-video-safari-profile").await;
    let safari = |mut value: Value| {
        let text = value.to_string().replace("firefox:agent", "safari:abc");
        value = serde_json::from_str(&text).unwrap();
        value
    };
    let bare = |mut value: Value| {
        let text = value.to_string().replace("firefox:agent", "abc");
        value = serde_json::from_str(&text).unwrap();
        value
    };
    let seen = super::test_support::answering(&core.shared, move |tool, args| match tool {
        "chrome_status" => safari(status()),
        "chrome_execute" if args["action"] == "recording.video.start" => bare(start_result(2)),
        "chrome_execute" if args["action"] == "capture.screenshot" => {
            bare(screenshot(&args["_approvedCapabilities"]))
        }
        "chrome_execute" if args["action"] == "recording.video.stop" => bare(stopped_result()),
        _ => panic!("unexpected {tool}: {args}"),
    });
    let answer = super::read_consent::send(
        &core.shared,
        "s",
        "chrome_execute",
        request("recording.video.start", json!({"tabId":7,"maxSeconds":2})),
        None,
    )
    .await
    .expect("a Safari recording with its own bare profile ID starts");
    assert_eq!(answer.value()["structuredContent"]["status"], "completed");
    assert!(
        encoder
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, VideoCommand::Frame { .. }))
    );
    // A different bare ID still fails: the match is exact on the UUID.
    assert!(
        !super::external_video::same_profile(&json!("abd"), &json!("safari:abc"))
            && !super::external_video::same_profile(&json!("abc"), &json!("chrome:abc"))
            && super::external_video::same_profile(
                &json!("firefox:agent"),
                &json!("firefox:agent")
            )
    );
    drop(seen);
    core.shutdown().await;
}
