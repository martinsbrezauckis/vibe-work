//! Native frame evaluation seam for browser refs. The engine supplies routing handles, never page messages.
//! Called by browser observation/input through the service's mount lock; unsupported engines refuse isolated reads.

use std::sync::Arc;

use crate::{EngineError, MountId, PreviewError, PreviewService};

/// Prevents a main-document operation from running in child frames while using native frame enumeration.
#[must_use]
pub fn main_script(script: &str) -> String {
    format!("if (window === window.top) {{\n{script}\n}} else {{ null; }}")
}

/// Selects exactly one native main frame; ambiguous or missing frame identity is never accepted.
///
/// # Errors
/// The native main document is unavailable or ambiguous.
pub fn main_answer(answers: Vec<FrameAnswer>) -> Result<FrameAnswer, EngineError> {
    let mut main = answers.into_iter().filter(|answer| answer.main);
    let answer = main
        .next()
        .ok_or_else(|| EngineError::Platform("the main document is unavailable".into()))?;
    if main.next().is_some() {
        return Err(EngineError::Platform(
            "the main document is ambiguous".into(),
        ));
    }
    Ok(answer)
}

/// The answer an all-frames evaluation gives for a child frame whose script could not run (a scriptless sandbox, a
/// frame destroyed meanwhile): the browser leaves that frame out of a read, and a checked click is refused while a
/// frame cannot report its events. A JSON string holding `{"unreadable":true}`.
pub const UNREADABLE_FRAME: &str = r#""{\"unreadable\":true}""#;

/// The answer of a child frame already removed from its page whose handle still answers from the detached document
/// (`WebKit` on macOS keeps such a handle for a while): no frame of the page, left out of reads and of a checked
/// click's frame registry. A JSON string holding `{"detached":true}`. Every attached child window has a parent, in
/// every script world, so no frame that is part of the page can answer it.
pub const DETACHED_FRAME: &str = r#""{\"detached\":true}""#;

/// Wraps an all-frames script so that a removed frame's handle answers [`DETACHED_FRAME`] instead of running it.
#[must_use]
pub fn detached_aware(script: &str) -> String {
    format!(
        "(() => {{ if (window !== window.top && window.parent === null) return JSON.stringify({{ detached: true }}); return ({script}); }})()"
    )
}

/// Why a checked native click is refused before any input while a frame on the page cannot report its events.
pub const UNREPORTING_FRAME_REFUSAL: &str =
    "Not run: a frame on this page cannot report a click's events; read the page again";

/// An answer from one native frame handle.
#[derive(Clone, Debug)]
pub struct FrameAnswer {
    /// Opaque native routing handle, scoped to this mount.
    pub id: String,
    /// Native registry lifecycle generation; changes on frame creation, removal or document replacement.
    /// None means the engine cannot supply this evidence for checked native input.
    pub generation: Option<u64>,
    /// Whether this is the top document.
    pub main: bool,
    /// URL supplied by the native frame API, independently of script output.
    pub url: String,
    /// Security origin captured before page scripts, when evaluation shares the page world.
    pub origin: Option<String>,
    /// JSON-encoded script result.
    pub value: String,
}

/// Admits only checked input backed by isolated frame state and an event receipt.
///
/// # Errors
/// Page-world validation or the older boolean-only input path cannot authorize a native click.
pub fn checked_frame_request(
    validation: &str,
    isolated: bool,
) -> Result<serde_json::Value, String> {
    if !isolated {
        return Err(
            "Not run: checked native clicks require isolated frame validation on this platform"
                .into(),
        );
    }
    let request: serde_json::Value = serde_json::from_str(validation)
        .map_err(|_| "Not run: checked native clicks require event-time target confirmation")?;
    if request["framePointValidation"] != true || request["receipt"].as_str().is_none() {
        return Err("Not run: checked native clicks require event-time target confirmation".into());
    }
    Ok(request)
}

/// Checks the native handles and boolean results of a host-built point validation request.
///
/// # Errors
/// A missing frame, changed node/geometry, malformed result or evaluation failure.
pub fn validate_frame_point(
    request: &serde_json::Value,
    answers: &[FrameAnswer],
) -> Result<(), String> {
    let ids = request["ids"]
        .as_array()
        .ok_or("invalid point validation")?;
    if ids.is_empty() || ids.len() > 9 {
        return Err("invalid point ancestry".into());
    }
    for (index, id) in ids.iter().enumerate() {
        let id = id.as_str().ok_or("invalid frame handle")?;
        let mut matching = answers.iter().filter(|a| a.id == id);
        let answer = matching.next().ok_or("frame is stale or detached")?;
        if matching.next().is_some() {
            return Err("frame identity is ambiguous".into());
        }
        let raw: serde_json::Value =
            serde_json::from_str(&answer.value).map_err(|_| "invalid point answer")?;
        let value: serde_json::Value = match raw {
            serde_json::Value::String(text) => {
                serde_json::from_str(&text).map_err(|_| "invalid point answer")?
            }
            other => other,
        };
        if value["ok"] != true {
            let reason = value["reason"].as_str().unwrap_or("frame document changed");
            // Only the fixed vocabulary emitted by our guard is shown, never page exception text.
            let known = [
                "viewport changed",
                "visual viewport changed",
                "point outside frame",
                "node or frame under point changed",
                "the page moved; read the page again",
                "target geometry changed",
                "target is not actionable",
                "transformed target",
                "authorization changed",
                "keyboard target cannot keep focus; read the page and choose its current ref",
                "native colour picker unsupported; use browser_read for the field ref, then browser_act type with #RRGGBB text",
            ];
            return Err(if known.contains(&reason) {
                reason
            } else {
                "frame document changed"
            }
            .into());
        }
        if value["armed"] != request["receipt"]
            || value["document"] != request["documents"][index]
            || value["armed"].as_str().is_none()
        {
            return Err("event-time target confirmation could not be armed".into());
        }
    }
    Ok(())
}

impl PreviewService {
    /// Evaluates in all live frames, or in one named frame. The same mount and input admission apply to both.
    /// `None` is returned only by engines without frame evaluation.
    ///
    /// # Errors
    /// A closed mount, withdrawn authority, stale frame or bounded native evaluation failure.
    pub async fn evaluate_frames_on_mount(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<Option<Vec<FrameAnswer>>, PreviewError> {
        self.frames_on_mount(id, frame, script, allowed, false)
            .await
    }

    /// [`Self::evaluate_frames_on_mount`] for an act's own input, which the engine runs as the person's gesture
    /// (`PreviewEngine::evaluate_frames_as_input`).
    ///
    /// # Errors
    /// As [`Self::evaluate_frames_on_mount`].
    pub async fn evaluate_input_frames_on_mount(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<Option<Vec<FrameAnswer>>, PreviewError> {
        self.frames_on_mount(id, frame, script, allowed, true).await
    }

    async fn frames_on_mount(
        &self,
        id: &MountId,
        frame: Option<String>,
        script: String,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
        input: bool,
    ) -> Result<Option<Vec<FrameAnswer>>, PreviewError> {
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id) {
            return Err(EngineError::NotMounted.into());
        }
        if allowed.is_some_and(|check| !check()) {
            return Err(EngineError::Platform(
                "Not run: page ownership ended before input.".into(),
            )
            .into());
        }
        self.touch_pool(&id.session_id);
        Ok(self
            .bounded(
                "frame evaluation",
                if input {
                    self.engine.evaluate_frames_as_input(id, frame, script)
                } else {
                    self.engine.evaluate_frames(id, frame, script)
                },
            )
            .await?)
    }
}
