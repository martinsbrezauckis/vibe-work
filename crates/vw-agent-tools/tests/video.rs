//! Video discovery and actual MCP image receipt at the stdio tool boundary.
#[cfg(feature = "video")]
use serde_json::{Value, json};
#[test]
#[cfg(not(feature = "video"))]
fn video_is_absent_without_feature() {
    assert!(
        !vw_agent_tools::mcp::tool_list()
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "video_frames")
    );
}
#[test]
#[cfg(all(feature = "video", any(windows, target_os = "macos")))]
fn video_reaches_mcp_as_image_content() {
    let list = vw_agent_tools::mcp::tool_list();
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "video_frames")
    );
    let input=json!({"id":1,"method":"tools/call","params":{"name":"video_frames","arguments":{"path":"clip.mp4","selection":{"kind":"timestamps","times_ms":[999]},"output":"frame"},"_meta":{"progressToken":"p1"}}}).to_string()+"\n";
    let mut output = Vec::new();
    vw_agent_tools::mcp::serve(input.as_bytes(), &mut output, |name:&str,_:Value| {
        assert_eq!(name,"video_frames");
        Ok(json!({"status":"ready","frame_count":1,"image_count":1,"vwImage":{"mimeType":"image/png","data":"iVBORw0KGgo="}}))
    }).unwrap();
    let messages: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2, "planned count must precede the image");
    assert_eq!(messages[0]["method"], "notifications/progress");
    assert_eq!(
        messages[0]["params"]["message"],
        "1 selected frames; 1 image"
    );
    let reply = &messages[1];
    assert_eq!(
        reply["result"]["content"][0],
        json!({"type":"image","mimeType":"image/png","data":"iVBORw0KGgo="})
    );
    assert!(
        reply["result"]["structuredContent"]
            .get("vwImage")
            .is_none()
    );
}

#[test]
fn video_catalog_follows_the_platform_and_feature() {
    let listed = vw_agent_tools::mcp::tool_list()
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "video_frames");
    assert_eq!(
        listed,
        cfg!(all(feature = "video", any(windows, target_os = "macos")))
    );
}

#[test]
#[cfg(all(feature = "video", target_os = "linux"))]
fn linux_video_refuses_without_progress_or_forwarding() {
    let input = json!({"id":1,"method":"tools/call","params":{"name":"video_frames","arguments":{"path":"clip.mp4","selection":{"kind":"timestamps","times_ms":[999]},"output":"frame"}}}).to_string()+"\n";
    let mut output = Vec::new();
    vw_agent_tools::mcp::serve(
        input.as_bytes(),
        &mut output,
        |_: &str, _: Value| -> Result<Value, String> {
            panic!("Linux video must not reach the core");
        },
    )
    .unwrap();
    let response: Value = serde_json::from_slice(&output).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("Not run: video review is not available on Linux yet."));
    assert!(text.contains("Ask for still images"));
}
