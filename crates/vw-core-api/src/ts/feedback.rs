//! Feedback TypeScript declarations, using ts-rs and the core wire types; called by the core TS module registry.
use ts_rs::{Config, TS};
pub(super) fn decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::FeedbackKind::decl(cfg),
        crate::FeedbackDraft::decl(cfg),
        crate::FeedbackSystem::decl(cfg),
        crate::FeedbackScreenshot::decl(cfg),
        crate::FeedbackReport::decl(cfg),
        crate::FeedbackView::decl(cfg),
        crate::FeedbackRequest::decl(cfg),
    ]
}
