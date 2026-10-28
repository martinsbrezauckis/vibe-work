//! Handbook and project TypeScript declarations for the core API generator.
//! Depends on ts-rs and the project API types; called by the parent module without changing declaration order.

use ts_rs::{Config, TS};

/// Project handbook types, in their own module for the same size reason (`Rect` comes from `preview.ts`).
pub(super) fn pages_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::PagesRequest::decl(cfg),
        crate::PagesResult::decl(cfg),
        crate::ProjectHub::decl(cfg),
        vw_pages::Hub::decl(cfg),
        vw_pages::HubGroup::decl(cfg),
        vw_pages::HubPage::decl(cfg),
        vw_pages::IndexHealth::decl(cfg),
        vw_pages::Validation::decl(cfg),
        vw_pages::Finding::decl(cfg),
        vw_pages::Severity::decl(cfg),
        vw_pages::PageFacts::decl(cfg),
        vw_pages::Relation::decl(cfg),
        vw_pages::Step::decl(cfg),
        vw_pages::Section::decl(cfg),
        vw_pages::Comment::decl(cfg),
        vw_pages::PagesSettings::decl(cfg),
        vw_pages::PickedSection::decl(cfg),
        vw_pages::Picked::decl(cfg),
        vw_pages::ViewerView::decl(cfg),
    ]
    .into_iter()
    .map(|decl| {
        decl.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    })
    .collect()
}

/// Handbook types in their own module so `pages.ts` stays under the size limit (`pages.ts` imports them): the
/// Projects overview's heads and Format 1.2's page inputs, answers and visual check.
pub(super) fn handbook_decls(cfg: &Config) -> Vec<String> {
    vec![
        vw_pages::ProjectCard::decl(cfg),
        vw_pages::Manifest::decl(cfg),
        vw_pages::PlanSummary::decl(cfg),
        vw_pages::HandbookState::decl(cfg),
        vw_pages::Input::decl(cfg),
        vw_pages::InputOption::decl(cfg),
        vw_pages::Answer::decl(cfg),
        vw_pages::AnswerValue::decl(cfg),
        vw_pages::InputAnswer::decl(cfg),
        vw_pages::PageAnswers::decl(cfg),
        vw_pages::VisualIssue::decl(cfg),
        vw_pages::VisualReport::decl(cfg),
    ]
}

/// The user's project groups, in their own module for the same size reason.
pub(super) fn projects_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::ProjectsRequest::decl(cfg),
        crate::ProjectsResult::decl(cfg),
        crate::ProjectView::decl(cfg),
    ]
}
