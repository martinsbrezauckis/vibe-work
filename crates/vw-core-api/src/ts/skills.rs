//! Skills declarations for the core TS generator; depends on the public Tools API view types.
use ts_rs::{Config, TS};
pub(super) fn decls(cfg: &Config) -> Vec<String> {
    let declarations = vec![
        crate::SkillsAction::decl(cfg),
        crate::SkillsRequest::decl(cfg),
        crate::SkillsResult::decl(cfg),
        crate::SkillsView::decl(cfg),
        crate::SkillRow::decl(cfg),
        crate::SkillDetail::decl(cfg),
        crate::SkillIssue::decl(cfg),
    ];
    declarations
        .into_iter()
        .map(|decl| {
            decl.lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}
