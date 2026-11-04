//! Feedback lifecycle owned by the core. Drafts stay in memory, preference alone is persisted; only Send does network I/O.
//! Called by typed commands and the agent draft tool; the latter can never confirm, alter diagnostics, or send.
use crate::core::Shared;
use crate::{
    Core, CoreError, ErrorCode, FeedbackDraft, FeedbackReport, FeedbackRequest, FeedbackView,
    Origin,
};
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct State {
    draft: Option<FeedbackDraft>,
    review: Option<FeedbackReport>,
    share: bool,
}
pub(crate) struct Feedback {
    state: tokio::sync::Mutex<State>,
    settings: Option<PathBuf>,
}
impl Feedback {
    pub(crate) fn new(config: &crate::CoreConfig) -> Self {
        let settings = config
            .secret_scan_data
            .as_ref()
            .filter(|_| cfg!(feature = "feedback"))
            .map(|p| p.join("settings.json"));
        let share = settings
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["feedbackShareSystem"].as_bool())
            .unwrap_or(false);
        Self {
            state: tokio::sync::Mutex::new(State {
                share,
                ..State::default()
            }),
            settings,
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
        system: crate::feedback_system::collect(core),
        available: crate::feedback_transport::endpoint().is_some(),
    }
}
impl Core {
    pub(crate) async fn feedback(
        &self,
        origin: Origin,
        request: FeedbackRequest,
    ) -> Result<FeedbackView, CoreError> {
        if !cfg!(feature = "feedback") {
            return Err(invalid("Feedback is unavailable in this build."));
        }
        if !matches!(
            request,
            FeedbackRequest::Read | FeedbackRequest::Open { .. }
        ) && crate::command_origin::debug_api_acts(&self.shared, origin)
        {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "Feedback needs the person's review and Send click in the window. Offer a draft and wait for their decision.",
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
                state.draft = Some(crate::feedback_redact::draft(&self.shared, draft, false)?);
                state.review = None;
            }
            FeedbackRequest::Review { draft } => {
                let draft = crate::feedback_redact::draft(&self.shared, draft, true)?;
                let report = FeedbackReport {
                    id: id()?,
                    draft: draft.clone(),
                    system: state.share.then(|| crate::feedback_system::collect(self)),
                };
                if serde_json::to_vec(&report).map_or(true, |v| v.len() > 16384) {
                    return Err(invalid(
                        "The report is too long. Shorten it and review again.",
                    ));
                }
                state.draft = Some(draft);
                state.review = Some(report);
            }
            FeedbackRequest::Share { share_system } => {
                state.share = share_system;
                if let Some(review) = state.review.as_mut() {
                    review.id = id()?;
                    review.system = share_system.then(|| crate::feedback_system::collect(self));
                }
                // A disk error never prevents sending with either choice. Only this boolean is saved.
                if let Some(path) = &self.shared.feedback.settings {
                    let update = serde_json::Map::from_iter([(
                        "feedbackShareSystem".into(),
                        share_system.into(),
                    )]);
                    if crate::write_app_settings(path, update).is_err() {
                        tracing::warn!("feedback sharing preference could not be saved");
                    }
                }
            }
            FeedbackRequest::Close => {
                state.draft = None;
                state.review = None;
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
                        "More private text was found. Go back and review the report again.",
                    ));
                }
                let url = crate::feedback_transport::endpoint()
                    .ok_or_else(|| invalid("Feedback sending is not configured in this build."))?;
                crate::feedback_transport::send(url, report).await?;
                state.draft = None;
                state.review = None;
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

#[cfg(all(test, not(feature = "feedback")))]
#[tokio::test]
async fn feedback_default_build_refuses_commands_and_omits_the_tool() {
    let core = crate::offline_core::start("feedback-disabled").await;
    assert!(
        core.feedback(Origin::Ui, FeedbackRequest::Read)
            .await
            .is_err()
    );
    assert!(!vw_agent_tools::wire::tools::SESSION_TOOLS.contains(&"feedback_draft"));
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
    let draft: FeedbackDraft = serde_json::from_value(args).map_err(|_| "Supply kind, title, description and remarks as plain text. No conversation or files are attached; include only the relevant reproduction.")?;
    let core = Core {
        shared: shared.clone(),
    };
    core.feedback(Origin::DebugApi, FeedbackRequest::Open { draft })
        .await
        .map_err(|error| error.message)?;
    Ok(
        serde_json::json!({"status":"draftOffered", "message":"The person can edit and review this draft in Vibe Work. Nothing has been sent. Wait for their Send decision; do not submit it through another channel."}),
    )
}
