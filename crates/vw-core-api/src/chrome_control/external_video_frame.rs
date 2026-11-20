//! One sampled frame of an external video: the helper screenshot, its protected-field guard, the page identity and whether the session is still alive.
use super::*;

pub(super) async fn frame(shared: &Arc<Shared>, capture: &Capture) -> Result<Frame, String> {
    let Capture {
        chat,
        session,
        identity,
        mode,
        started,
        max_seconds,
        bridge,
    } = capture;
    let operation = shared.chrome_control.external_video.operation(session);
    let Ok(_guard) = operation.try_lock() else {
        return Ok(Frame::Omitted);
    };
    alive(shared, chat, session, identity, mode).await?;
    let tab = identity["tabId"]
        .as_u64()
        .ok_or("The recording has no tab")?;
    let request = json!({"sessionId":session,"action":"capture.screenshot","args":{"tabId":tab,"format":"png","maxWidth":800,"maxHeight":600}});
    let answer = tokio::time::timeout(
        Duration::from_secs(12),
        super::super::escalation::video_frame(shared, chat, request),
    )
    .await
    .map_err(|_| "The screenshot did not finish")??;
    let result = answer.value();
    let body = &result["structuredContent"];
    if body["status"] != "completed" || result["isError"] == true {
        let code = body["error"]["code"].as_str().unwrap_or_default();
        if matches!(code, "TAB_BUSY" | "OPERATION_BUSY") {
            alive(shared, chat, session, identity, mode).await?;
            return Ok(Frame::Omitted);
        }
        if code == "CAPABILITY_REQUIRED"
            && body["error"]["details"]["capability"] == "auth.secrets"
            && body["error"]["details"]["operation"]
                == "capturing a page that shows a password, PIN or card value"
        {
            alive(shared, chat, session, identity, mode).await?;
            return Ok(Frame::Protected);
        }
        // The helper's error code is a fixed protocol word, safe to log.
        tracing::warn!(code, "browser video frame screenshot refused");
        return Err("The guarded screenshot was refused".into());
    }
    let png = guarded_png(result, identity)?;
    alive(shared, chat, session, identity, mode).await?;
    if started.elapsed() >= Duration::from_secs(*max_seconds) {
        return Ok(Frame::Omitted);
    }
    let recording = identity["recordingId"]
        .as_str()
        .ok_or("The recording ID is unavailable")?;
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        bridge.send(VideoCommand::Frame {
            recording: recording.to_owned(),
            png,
            at_seconds: started
                .elapsed()
                .min(Duration::from_millis(*max_seconds * 1000 - 33))
                .as_secs_f64(),
        }),
    )
    .await
    .map_err(|_| "The video encoder did not accept the frame")??;
    if !matches!(answer, VideoReply::Added) {
        return Err("The video encoder refused the frame".into());
    }
    alive(shared, chat, session, identity, mode).await?;
    Ok(Frame::Added)
}

pub(super) fn guarded_png(result: &Value, identity: &Value) -> Result<Vec<u8>, String> {
    let body = &result["structuredContent"];
    if [
        "profileId",
        "browserSessionId",
        "leaseEpoch",
        "policyRevision",
        "documentId",
        "tabId",
        "recordingId",
    ]
    .iter()
    .any(|field| body["data"]["captureIdentity"][*field] != identity[*field])
    {
        return Err("The guarded screenshot changed recording identity".into());
    }
    let image = result["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block["type"] == "image" && block["mimeType"] == "image/png")
        })
        .and_then(|block| block["data"].as_str())
        .ok_or("The browser returned no guarded PNG")?;
    if image.len() > MAX_PNG * 4 / 3 + 8 {
        return Err("The video frame exceeds the screenshot limit".into());
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(image)
        .map_err(|_| "The browser returned invalid PNG bytes")?;
    if png.len() > MAX_PNG || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The browser returned invalid PNG bytes".into());
    }
    Ok(png)
}

pub(super) async fn alive(
    shared: &Arc<Shared>,
    chat: &str,
    session: &str,
    identity: &Value,
    mode: &str,
) -> Result<(), String> {
    if super::super::check::mode(shared, chat).as_deref() != Some(mode) {
        return Err("The chat mode changed".into());
    }
    let request = json!({"sessionId":session,"action":"capture.screenshot","args":{"tabId":identity["tabId"]}});
    super::super::check::authorize_read(shared, chat, "chrome_execute", &request, None)?;
    super::super::gateway::validate_session(shared, chat, "chrome_execute", &request)?;
    let status = shared
        .chrome_control
        .owner
        .call("chrome_status", json!({}))
        .await;
    let body = &status.value()["structuredContent"];
    let row = body["sessions"]
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row["sessionId"] == session
                    && same_profile(&identity["profileId"], &row["profileId"])
            })
        })
        .ok_or_else(|| {
            // Diagnostic: identifiers and states only (no page content).
            let rows = body["sessions"].as_array().map_or(0, Vec::len);
            let row_profile = body["sessions"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["sessionId"] == session))
                .map(|row| row["profileId"].to_string())
                .unwrap_or_default();
            let identity_profile = identity["profileId"].to_string();
            let is_error = status.value()["isError"].to_string();
            tracing::warn!(
                rows,
                row_profile = %row_profile,
                identity_profile = %identity_profile,
                is_error = %is_error,
                "browser video session row missing"
            );
            "The browser session ended"
        })?;
    if matches!(
        row["state"].as_str(),
        Some("paused" | "stopped" | "disconnected" | "releasing" | "idle")
    ) {
        return Err("The browser is no longer recording".into());
    }
    if row["state"] != "active"
        || row["leaseEpoch"] != identity["leaseEpoch"]
        || row["policyRevision"] != identity["policyRevision"]
        || row["target"]["tabId"] != identity["tabId"]
        || row["target"]["documentId"] != identity["documentId"]
    {
        return Err("The browser recording target changed".into());
    }
    let profile = body["profiles"]
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                same_profile(&identity["profileId"], &row["profileId"]) && row["connected"] == true
            })
        })
        .ok_or("The browser profile disconnected")?;
    if !profile["extensionCapabilities"]
        .as_array()
        .is_some_and(|caps| caps.contains(&json!("page.video.frames.v1")))
    {
        return Err("The browser video capability ended".into());
    }
    if profile["browserSessionId"] != identity["browserSessionId"] {
        return Err("The browser connection changed".into());
    }
    Ok(())
}

pub(super) fn complete(answer: &owner::Answer) -> Result<&Value, String> {
    let value = answer.value();
    if value["isError"] == true || value["structuredContent"]["status"] != "completed" {
        return Err("The browser did not confirm the video action".into());
    }
    Ok(&value["structuredContent"]["data"])
}

pub(super) fn identity(data: &Value) -> Result<Value, String> {
    if ["profileId", "browserSessionId", "documentId", "recordingId"]
        .iter()
        .any(|field| data[*field].as_str().is_none_or(str::is_empty))
        || ["leaseEpoch", "policyRevision", "tabId"]
            .iter()
            .any(|field| data[*field].as_u64().is_none())
        || data["tabId"]
            .as_u64()
            .is_some_and(|tab| tab > i32::MAX as u64)
    {
        return Err("The browser video has no exact identity".into());
    }
    Ok(data.clone())
}
