use super::super::{check, escalation};
use super::{
    Active, Arc, Capture, Consent, Duration, Frame, ID_FIELDS, Instant, MAX_FRAMES, MAX_WEBM,
    PoisonError, Shared, Stats, Value, VideoBridge, VideoCommand, VideoReply, alive,
    cancel_encoder, complete, discard_helper, frame, identity, json, owner, prepare_name, sample,
    save, valid_filename, watch,
};

pub(super) async fn start(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    let args = request["args"]
        .as_object()
        .ok_or("The recording Start arguments are invalid")?;
    if args
        .keys()
        .any(|key| !matches!(key.as_str(), "tabId" | "maxSeconds"))
        || args
            .get("tabId")
            .is_some_and(|value| value.as_u64().is_none_or(|tab| tab > i32::MAX as u64))
    {
        return Err("The recording Start arguments are invalid".into());
    }
    if args.get("maxSeconds").is_some_and(|value| {
        !value
            .as_u64()
            .is_some_and(|seconds| (1..=300).contains(&seconds))
    }) {
        return Err("maxSeconds must be a whole number from 1 to 300.".into());
    }
    let session = request["sessionId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("A browser session is required")?;
    let _reservation = shared.chrome_control.external_video.reserve(session)?;
    let bridge = shared.chrome_control.external_video.bridge()?;
    let mode =
        check::mode(shared, chat).ok_or(vw_vocabulary::texts::BROWSER_VIDEO_ACCESS_MODE_MISSING)?;
    let (mut answer, identity, max_seconds) =
        match helper_start(shared, chat, request, consent).await? {
            Started::Recording(answer, identity, seconds) => (answer, identity, seconds),
            // The browser's own refusal (code and reason) reaches the agent like any other action's.
            Started::Refused(answer) => return Ok(answer),
        };
    let recording = identity["recordingId"]
        .as_str()
        .ok_or("The browser video has no recording ID")?
        .to_owned();
    let start_result = tokio::time::timeout(
        Duration::from_secs(10),
        bridge.send(VideoCommand::Start {
            recording: recording.clone(),
        }),
    )
    .await;
    if !matches!(start_result, Ok(Ok(VideoReply::Ready))) {
        cancel_encoder(&bridge, &recording).await;
        discard_helper(shared, chat, request).await;
        return Err("The app's video encoder is unavailable; use an action GIF instead.".into());
    }
    let started = Instant::now();
    let capture = Capture {
        chat: chat.to_owned(),
        session: session.to_owned(),
        identity: identity.clone(),
        mode: mode.clone(),
        started,
        max_seconds,
        bridge,
    };
    let first = frame(shared, &capture).await;
    if !matches!(first, Ok(Frame::Added)) {
        cancel_encoder(&capture.bridge, &recording).await;
        discard_helper(shared, chat, request).await;
        return Err(first_frame_refusal(&first).into());
    }
    if let Err(error) = alive(shared, chat, session, &identity, &mode).await {
        cancel_encoder(&capture.bridge, &recording).await;
        discard_helper(shared, chat, request).await;
        return Err(error);
    }
    let (cancel, receiver) = watch::channel(0);
    let worker = tokio::spawn(sample(Arc::downgrade(shared), capture, receiver));
    shared
        .chrome_control
        .external_video
        .active
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            session.to_owned(),
            Active {
                chat: chat.to_owned(),
                start: identity.clone(),
                mode: mode.clone(),
                started,
                max_seconds,
                cancel,
                worker,
            },
        );
    answer.value_mut()["structuredContent"]["data"]["frames"] = json!(1);
    Ok(answer)
}

/// The Start refusal for a first frame that was not added; logs only fixed app-side reasons, never page content.
fn first_frame_refusal(first: &Result<Frame, String>) -> &'static str {
    if let Err(reason) = first {
        tracing::warn!(%reason, "browser video first frame failed");
    } else {
        tracing::warn!("browser video first frame was busy or protected");
    }
    if matches!(first, Ok(Frame::Protected)) {
        super::PROTECTED_START
    } else {
        "The guarded video frame could not be captured; use an action GIF instead."
    }
}

/// A helper Start either began a guarded recording or refused with its own receipt.
enum Started {
    Recording(owner::Answer, Value, u64),
    Refused(owner::Answer),
}

async fn helper_start(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<Started, String> {
    let answer =
        match escalation::send(shared, chat, "chrome_execute", request.clone(), consent).await {
            Ok(answer) => answer,
            Err(error) => {
                discard_helper(shared, chat, request).await;
                return Err(error);
            }
        };
    let value = answer.value();
    if value["isError"] == true || value["structuredContent"]["error"].is_object() {
        discard_helper(shared, chat, request).await;
        return Ok(Started::Refused(answer));
    }
    let parsed = (|| {
        let data = complete(&answer)?;
        if data["appEncoder"] != true || data["state"] != "recording" {
            return Err("The browser did not start a guarded video".into());
        }
        let max_seconds = data["maxSeconds"]
            .as_u64()
            .filter(|n| (1..=300).contains(n))
            .ok_or("The browser did not confirm the video limit")?;
        Ok((identity(data)?, max_seconds))
    })();
    match parsed {
        Ok((identity, seconds)) => Ok(Started::Recording(answer, identity, seconds)),
        Err(error) => {
            discard_helper(shared, chat, request).await;
            Err(error)
        }
    }
}

pub(super) async fn stop(
    shared: &Arc<Shared>,
    chat: &str,
    request: &Value,
    consent: Option<&Consent>,
) -> Result<owner::Answer, String> {
    let session = request["sessionId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("A browser session is required")?;
    let file_name = stop_name(shared, chat, request)?;
    let (mut active, _teardown) = shared.chrome_control.external_video.take_for_stop(
        session,
        chat,
        &request["args"]["tabId"],
    )?;
    let bridge = shared.chrome_control.external_video.bridge()?;
    active.cancel.send_replace(1);
    let Ok(Ok(stats)) = tokio::time::timeout(Duration::from_secs(15), &mut active.worker).await
    else {
        active.worker.abort();
        cancel_encoder(
            &bridge,
            active.start["recordingId"].as_str().unwrap_or_default(),
        )
        .await;
        discard_helper(shared, chat, request).await;
        return Err("The video worker did not stop".into());
    };
    let recording = active.start["recordingId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if active.chat != chat
        || stats.failed
        || alive(shared, chat, session, &active.start, &active.mode)
            .await
            .is_err()
    {
        cancel_encoder(&bridge, &recording).await;
        discard_helper(shared, chat, request).await;
        return Err("The recording was discarded after browser access changed.".into());
    }
    let answer =
        match escalation::send(shared, chat, "chrome_execute", request.clone(), consent).await {
            Ok(answer) => answer,
            Err(error) => {
                cancel_encoder(&bridge, &recording).await;
                return Err(error);
            }
        };
    let data = match complete(&answer) {
        Ok(data) => data,
        Err(error) => {
            cancel_encoder(&bridge, &recording).await;
            return Err(error);
        }
    };
    let same = ID_FIELDS
        .iter()
        .all(|field| data[*field] == active.start[*field]);
    if !same || data["appEncoder"] != true {
        cancel_encoder(&bridge, &recording).await;
        return Err("The browser could not confirm this recording's identity.".into());
    }
    if request["args"]["discard"] == true || data["state"] == "discarded" {
        cancel_encoder(&bridge, &recording).await;
        return Ok(answer);
    }
    if data["state"] != "stopped" {
        cancel_encoder(&bridge, &recording).await;
        return Err("The browser did not confirm Stop".into());
    }
    let (bytes, frames, duration_ms) = finish_encoder(&bridge, &recording, &active, &stats).await?;
    if let Err(error) = alive(shared, chat, session, &active.start, &active.mode).await {
        cancel_encoder(&bridge, &recording).await;
        return Err(error);
    }
    let file = save(
        shared,
        chat,
        file_name
            .as_deref()
            .ok_or("The recording filename is unavailable")?,
        &bytes,
    )?;
    Ok(saved_answer(
        answer,
        &stats,
        file,
        frames,
        duration_ms,
        bytes.len(),
    ))
}

fn stop_name(shared: &Shared, chat: &str, request: &Value) -> Result<Option<String>, String> {
    if request["args"].as_object().is_none_or(|args| {
        args.keys()
            .any(|key| !matches!(key.as_str(), "tabId" | "filename" | "discard"))
    }) || request["args"]
        .get("discard")
        .is_some_and(|value| !value.is_boolean())
        || request["args"]
            .get("tabId")
            .is_some_and(|value| value.as_u64().is_none_or(|tab| tab > i32::MAX as u64))
        || request["args"]
            .get("filename")
            .is_some_and(|value| value.as_str().is_none_or(|name| !valid_filename(name)))
    {
        return Err("The recording Stop arguments are invalid".into());
    }
    let file_name = if request["args"]["discard"] == true {
        None
    } else {
        Some(prepare_name(shared, chat, request)?)
    };
    Ok(file_name)
}

async fn finish_encoder(
    bridge: &Arc<dyn VideoBridge>,
    recording: &str,
    active: &Active,
    stats: &Stats,
) -> Result<(Vec<u8>, usize, u64), String> {
    let finished = tokio::time::timeout(
        Duration::from_secs(15),
        bridge.send(VideoCommand::Finish {
            recording: recording.to_owned(),
            at_seconds: active
                .started
                .elapsed()
                .min(Duration::from_secs(active.max_seconds))
                .as_secs_f64(),
        }),
    )
    .await;
    let Ok(Ok(finished)) = finished else {
        cancel_encoder(bridge, recording).await;
        return Err("The video encoder did not finish".into());
    };
    let VideoReply::Finished {
        bytes,
        frames,
        duration_ms,
    } = finished
    else {
        cancel_encoder(bridge, recording).await;
        return Err("The video encoder did not return a WebM".into());
    };
    if frames == 0
        || frames > MAX_FRAMES
        || frames > stats.frames
        || duration_ms > active.max_seconds * 1000
        || bytes.len() > MAX_WEBM
        || !bytes.starts_with(b"\x1a\x45\xdf\xa3")
    {
        cancel_encoder(bridge, recording).await;
        return Err("The video encoder returned invalid WebM bytes".into());
    }
    Ok((bytes, frames, duration_ms))
}

fn saved_answer(
    answer: owner::Answer,
    stats: &Stats,
    file: Value,
    frames: usize,
    duration_ms: u64,
    size: usize,
) -> owner::Answer {
    let mut result = answer.into_value();
    result["structuredContent"]["data"]["state"] = json!("saved");
    result["structuredContent"]["data"]["frames"] = json!(frames);
    result["structuredContent"]["data"]["omitted"] = json!(stats.omitted);
    result["structuredContent"]["data"]["limit"] = json!(stats.limit);
    result["structuredContent"]["data"]["durationMs"] = json!(duration_ms);
    result["structuredContent"]["data"]["bytes"] = json!(size);
    result["structuredContent"]["data"]["mimeType"] = json!("video/webm");
    result["structuredContent"]["data"]["file"] = file;
    result["content"] = json!([{"type":"text","text":result["structuredContent"].to_string()}]);
    owner::Answer::Helper(result)
}
