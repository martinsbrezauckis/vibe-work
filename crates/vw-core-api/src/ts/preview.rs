//! Browser and preview TypeScript declarations for the core API generator.
//! Depends on ts-rs and the native public types; called by `ts::typescript_modules`.

use ts_rs::{Config, TS};

/// Preview pane types, in their own module for the same size reason.
pub(super) fn preview_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::PreviewRequest::decl(cfg),
        crate::PreviewResult::decl(cfg),
        crate::preview_view::BrowserOverlayAction::decl(cfg),
        crate::PreviewTarget::decl(cfg),
        crate::preview_view::PickTarget::decl(cfg),
        crate::BusyPortView::decl(cfg),
        crate::BusyOption::decl(cfg),
        vw_preview::PreviewView::decl(cfg),
        vw_preview::PageTab::decl(cfg),
        vw_preview::PageTabKind::decl(cfg),
        vw_preview::PageTabOwner::decl(cfg),
        vw_preview::EnginePoolSnapshot::decl(cfg),
        vw_preview::PreviewState::decl(cfg),
        vw_preview::PreviewServerState::decl(cfg),
        vw_preview::VisibilityReport::decl(cfg),
        vw_preview::DeviceWidth::decl(cfg),
        vw_preview::Rect::decl(cfg),
        vw_preview::ConsoleEntry::decl(cfg),
        vw_preview::ConsoleLevel::decl(cfg),
        vw_preview::DownloadView::decl(cfg),
        vw_preview::DownloadState::decl(cfg),
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

/// Browser command, state and receipt types in a separate generated module.
pub(super) fn browser_decls(cfg: &Config) -> Vec<String> {
    vec![
        // The agent browser.
        crate::BrowserRequest::decl(cfg),
        crate::BrowserResult::decl(cfg),
        crate::BrowserShot::decl(cfg),
        crate::browser_view::SecretChoiceView::decl(cfg),
        vw_browser::delegation::BrowserPermissions::decl(cfg),
        vw_browser::delegation::DelegatedAct::decl(cfg),
        vw_browser::delegation::DelegationAuthority::decl(cfg),
        vw_browser::delegation::DelegationReceipt::decl(cfg),
        vw_preview::accounts::Provider::decl(cfg),
        vw_preview::accounts::ProviderAccount::decl(cfg),
        vw_browser::BrowserState::decl(cfg),
        vw_browser::LogEntry::decl(cfg),
        vw_browser::Actor::decl(cfg),
        vw_browser::Outcome::decl(cfg),
        vw_browser::annotate::Annotation::decl(cfg),
        vw_browser::annotate::ViewRect::decl(cfg),
        vw_browser::annotate::RecheckItem::decl(cfg),
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

/// Protected-folder review declarations; histories carry these through the core's JSON boundary.
pub(super) fn proposal_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::ProtectedChangeView::decl(cfg),
        crate::ProtectedChangeStatus::decl(cfg),
        crate::ProtectedFileView::decl(cfg),
        crate::ProtectedChangeProblem::decl(cfg),
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
