//! Git TypeScript declarations for the core binding generator; called by ts.rs.
//! Depends on ts-rs and the Rust API types; no generated file is edited here.
use super::trim_decl_lines;
use ts_rs::{Config, TS};

pub(super) fn git_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::GitResult::decl(cfg),
        crate::PreparedKind::decl(cfg),
        crate::GitLocation::decl(cfg),
        crate::GitRepoView::decl(cfg),
        crate::GitStatusView::decl(cfg),
        crate::CommitPlanView::decl(cfg),
        crate::DraftSource::decl(cfg),
        crate::PushPlanView::decl(cfg),
        crate::PrVia::decl(cfg),
        crate::PrPlanView::decl(cfg),
        crate::PrCreatedView::decl(cfg),
        crate::PrSource::decl(cfg),
        crate::PrStatusView::decl(cfg),
        crate::BringPlanView::decl(cfg),
        crate::GitSettings::decl(cfg),
        crate::GitIdentitySettings::decl(cfg),
        crate::GitIdentitySource::decl(cfg),
        vw_git::state::BranchState::decl(cfg),
        vw_git::state::HeadState::decl(cfg),
        vw_git::state::InProgress::decl(cfg),
        vw_git::state::RemoteInfo::decl(cfg),
        vw_git::state::Upstream::decl(cfg),
        vw_git::state::BaseInfo::decl(cfg),
        vw_git::forge::Forge::decl(cfg),
        vw_git::forge::ForgeKind::decl(cfg),
        vw_git::commit::CommitFile::decl(cfg),
        vw_git::commit::CommitStatus::decl(cfg),
        vw_git::hooks::HookInfo::decl(cfg),
        vw_git::identity::CommitIdentity::decl(cfg),
        vw_git::push::PushPlan::decl(cfg),
        vw_git::merge::BringPlan::decl(cfg),
        vw_git::merge::BringKind::decl(cfg),
        vw_git::pr::PrCard::decl(cfg),
        vw_git::pr::PrState::decl(cfg),
        vw_git::pr::Review::decl(cfg),
        vw_git::pr::Mergeable::decl(cfg),
        vw_git::pr::Checks::decl(cfg),
        vw_git::pr::CheckItem::decl(cfg),
    ]
    .into_iter()
    .map(|decl| trim_decl_lines(&decl))
    .collect()
}
