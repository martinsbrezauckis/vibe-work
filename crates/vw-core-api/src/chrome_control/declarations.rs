//! TypeScript declarations for browser commands and views; used by the core type generator.
use super::{
    BrowserProfile, ChatBrowserProfile, ChromeControlRequest, ChromeControlView, ChromeSelection,
    ChromeSetupView, ChromeShareKind, ChromeTab, ChromeWorkView, PairingState, ProfileState,
    SafariView, addon, browser, setup,
};

pub(crate) fn declarations(config: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        ChromeControlRequest::decl(config),
        ChromeControlView::decl(config),
        BrowserProfile::decl(config),
        ChatBrowserProfile::decl(config),
        ProfileState::decl(config),
        SafariView::decl(config),
        ChromeWorkView::decl(config),
        ChromeSelection::decl(config),
        ChromeShareKind::decl(config),
        ChromeTab::decl(config),
        ChromeSetupView::decl(config),
        setup::SetupState::decl(config),
        PairingState::decl(config),
        browser::BrowserIssue::decl(config),
        addon::AddonView::decl(config),
        addon::AddonState::decl(config),
        addon::AddonAction::decl(config),
    ]
}
