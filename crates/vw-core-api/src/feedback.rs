//! Feedback lifecycle owned by the core. Drafts and attachment consent stay in memory; only Send does network I/O.
//! Called by typed commands and the agent draft tool; the latter can never confirm, alter diagnostics, or send.
use crate::core::Shared;
use crate::{
    Core, CoreError, ErrorCode, FeedbackDraft, FeedbackReport, FeedbackRequest, FeedbackView,
    Origin,
};
use std::fmt::Write;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct State {
    draft: Option<FeedbackDraft>,
    review: Option<FeedbackReport>,
    share: bool,
    system: Option<crate::FeedbackSystem>,
    screenshot: Option<crate::FeedbackScreenshot>,
    add_screenshot: bool,
    screenshot_error: Option<String>,
}
pub(crate) type CaptureAnswer =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send>>;
pub(crate) type Capture = Box<dyn Fn() -> CaptureAnswer + Send + Sync>;
pub(crate) struct Feedback {
    state: tokio::sync::Mutex<State>,
    capture: std::sync::OnceLock<Capture>,
}
impl Feedback {
    pub(crate) fn new(_config: &crate::CoreConfig) -> Self {
        Self {
            state: tokio::sync::Mutex::new(State::default()),
            capture: std::sync::OnceLock::new(),
        }
    }
}
fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidParams, message)
}
fn id() -> Result<String, CoreError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        CoreError::new(
            ErrorCode::Internal,
            "Could not prepare the report. Try again.",
        )
    })?;
    Ok(bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    }))
}
fn view(core: &Core, state: &State) -> FeedbackView {
    FeedbackView {
        draft: state.draft.clone(),
        review: state.review.clone(),
        share_system: state.share,
        system: state
            .system
            .clone()
            .unwrap_or_else(|| crate::feedback_system::collect(core)),
        screenshot: state.screenshot.clone(),
        add_screenshot: state.add_screenshot,
        screenshot_error: state.screenshot_error.clone(),
        available: crate::feedback_transport::endpoint().is_some(),
    }
}
fn prepare(core: &Core, state: &mut State, draft: FeedbackDraft) -> Result<(), CoreError> {
    let draft = crate::feedback_redact::draft(&core.shared, draft, true)?;
    let mut report = FeedbackReport {
        id: id()?,
        draft: draft.clone(),
        system: state.share.then(|| {
            state
                .system
                .clone()
                .unwrap_or_else(|| crate::feedback_system::collect(core))
        }),
        screenshot: state.screenshot.clone(),
    };
    // Identical preparation is a retry, not a new report; keep the relay's immutable id.
    if let Some(previous) = state.review.as_ref().filter(|previous| {
        previous.draft == report.draft
            && previous.system == report.system
            && previous.screenshot == report.screenshot
    }) {
        report.id.clone_from(&previous.id);
    }
    if !crate::feedback_transport::within_limits(&report) {
        return Err(invalid(
            "The report is too long. Shorten it and review again.",
        ));
    }
    state.draft = Some(draft);
    state.review = Some(report);
    Ok(())
}
impl Core {
    /// Installs the host's secret-safe native app capture. The typed API accepts consent only, never image bytes.
    /// A host without a window leaves this unset and reports capture as unavailable.
    pub fn set_feedback_capture<F, Fut>(&self, capture: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<u8>, String>> + Send + 'static,
    {
        let _ = self
            .shared
            .feedback
            .capture
            .set(Box::new(move || Box::pin(capture())));
    }
    pub(crate) async fn feedback(
        &self,
        origin: Origin,
        request: FeedbackRequest,
    ) -> Result<FeedbackView, CoreError> {
        if !matches!(
            request,
            FeedbackRequest::Read | FeedbackRequest::Open { .. }
        ) && crate::command_origin::debug_api_acts(&self.shared, origin)
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "Feedback needs the user's review and Send click in the window. Offer a draft and wait for their decision.",
            ));
        }
        crate::title_guard::refresh(&self.shared).await;
        let mut state = self.shared.feedback.state.lock().await;
        match request {
            FeedbackRequest::Read => {}
            FeedbackRequest::Open { draft } => {
                if state.draft.is_some() {
                    return Err(invalid(
                        "A feedback draft is already open. Finish or cancel it first.",
                    ));
                }
                let draft = crate::feedback_redact::draft(&self.shared, draft, false)?;
                *state = State {
                    draft: Some(draft),
                    system: Some(crate::feedback_system::collect(self)),
                    ..State::default()
                };
            }
            FeedbackRequest::Review { draft } => prepare(self, &mut state, draft)?,
            FeedbackRequest::Share { share_system } => {
                state.share = share_system;
                let system = state
                    .system
                    .get_or_insert_with(|| crate::feedback_system::collect(self))
                    .clone();
                if let Some(review) = state.review.as_mut() {
                    review.id = id()?;
                    review.system = share_system.then_some(system);
                }
            }
            FeedbackRequest::Screenshot { include } => {
                if state.draft.is_none() {
                    return Err(invalid("Open feedback before adding a screenshot."));
                }
                state.add_screenshot = include;
                state.screenshot = None;
                state.screenshot_error = None;
                state.review = None;
                if include {
                    match crate::feedback_screenshot::capture(self.shared.feedback.capture.get())
                        .await
                    {
                        Ok(image) => state.screenshot = Some(image),
                        Err(message) => state.screenshot_error = Some(message),
                    }
                }
            }
            FeedbackRequest::Close => {
                *state = State::default();
            }
            FeedbackRequest::Send { review_id } => {
                let report = state
                    .review
                    .as_ref()
                    .filter(|r| r.id == review_id)
                    .ok_or_else(|| invalid("Review the report before sending."))?;
                // A newly unlocked vault may reveal a value that was unknown at preview time.
                let redacted =
                    crate::feedback_redact::draft(&self.shared, report.draft.clone(), true)?;
                if redacted != report.draft {
                    return Err(invalid(
                        "More private text was found. Check the form and send again.",
                    ));
                }
                let url = crate::feedback_transport::endpoint()
                    .ok_or_else(|| invalid("Feedback sending is not configured in this build."))?;
                crate::feedback_transport::send(url, report).await?;
                *state = State::default();
            }
        }
        let view = view(self, &state);
        let _ = self
            .shared
            .events
            .send(crate::CoreEvent::Feedback { view: view.clone() });
        Ok(view)
    }
}

#[cfg(test)]
#[tokio::test]
async fn feedback_default_build_reads_commands_and_lists_the_tool() {
    let core = crate::offline_core::start("feedback-enabled").await;
    assert!(
        core.feedback(Origin::Ui, FeedbackRequest::Read)
            .await
            .is_ok()
    );
    assert!(vw_agent_tools::wire::tools::SESSION_TOOLS.contains(&"feedback_draft"));
    core.shutdown().await;
}
/// The only feedback operation offered to agents: redacted prose, no system fields, no send capability.
pub(crate) async fn offer(
    shared: &Arc<Shared>,
    session: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if shared.store.view(session).is_none() {
        return Err("The session is not open. Open it before offering feedback.".into());
    }
    // Remarks are optional, as in the dialog: an absent field opens with empty remarks.
    let mut args = args;
    if let Some(fields) = args.as_object_mut() {
        fields
            .entry("remarks")
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    let draft: FeedbackDraft = serde_json::from_value(args).map_err(|_| "Supply kind, title and description as plain text; remarks are optional. No conversation or files are attached; include only the relevant reproduction.")?;
    let core = Core {
        shared: shared.clone(),
    };
    core.feedback(Origin::DebugApi, FeedbackRequest::Open { draft })
        .await
        .map_err(|error| error.message)?;
    Ok(
        serde_json::json!({"status":"draftOffered", "message":"The user can edit and review this draft in Vibe Work. Nothing has been sent. Wait for their Send decision; do not submit it through another channel."}),
    )
}
