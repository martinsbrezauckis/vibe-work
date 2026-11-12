//! Core API type declarations grouped for the TypeScript generator.
//! Depends on ts-rs and the Rust API types; called by the existing module/import assembler.
//! Declaration ordering and serialization derive from the same types as before.
use crate::{
    AlertKind, AnomalyCountsView, AnomalyRecordView, AnomalyReport, CallbackInfo, ChannelView,
    ChannelsReport, Command, CommandResult, CoreError, CoreEvent, CoreStatus, DeathView,
    DebugApiFlag, DocumentRefusal, DocumentRefusalKind, DocumentSource, DocumentView,
    DocumentsRequest, DocumentsResult, ErrorCode, InstructionsDoc, InstructionsRequest,
    InstructionsResult, InstructionsScope, KeptUnitView, KeptView, KeyMetadataView, KeyRequest,
    KeySourceView, KeyStatusView, Origin, ProjectInstructionFile, ProjectInstructionsRequest,
    ProjectInstructionsResult, RateLimitsView, RecoveringView, SecretInput, ServerReport,
    ServerStatus, SessionChange, SessionRowView, SessionSnapshot, StuckView, SubagentCard,
    TerminalEventView, TerminalOutputView, TerminalRequest, TerminalResult, TerminalStateView,
    TerminalView, VaultChoice, VibeKeySource, VibeKeyView,
};
use crate::{
    CatalogLoad, ConnectorView, ConnectorsRequest, ConnectorsResult, ConnectorsView, ServerView,
    ToolView,
};
use crate::{
    ChangesMode, ChangesRequest, ChangesResult, ChangesSubagent, ChangesView, FolderSession,
    TurnChanges, TurnFile, UndoPreview,
};
use crate::{DictationMode, DictationUpdate, SpeechRequest, SpeechResult, VoiceView};
use ts_rs::{Config, TS};

/// Connector and MCP server types, in their own module.
pub(super) fn connectors_decls(cfg: &Config) -> Vec<String> {
    vec![
        ConnectorsRequest::decl(cfg),
        ConnectorsResult::decl(cfg),
        ConnectorsView::decl(cfg),
        CatalogLoad::decl(cfg),
        ConnectorView::decl(cfg),
        ServerView::decl(cfg),
        ToolView::decl(cfg),
        crate::NewServerSecret::decl(cfg),
        crate::ServerKeyView::decl(cfg),
    ]
}

/// Marketplace types, in their own module.
pub(super) fn market_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::MarketRequest::decl(cfg),
        crate::MarketResult::decl(cfg),
        crate::MarketView::decl(cfg),
        crate::LocalPluginView::decl(cfg),
        crate::PluginServerView::decl(cfg),
        crate::CategoryView::decl(cfg),
        crate::MarketEntryView::decl(cfg),
        crate::InstalledView::decl(cfg),
        crate::SessionPluginView::decl(cfg),
        crate::ComponentView::decl(cfg),
        crate::SkillStateView::decl(cfg),
        crate::DroppedView::decl(cfg),
        crate::InstallProgress::decl(cfg),
        vw_market::Entry::decl(cfg),
        vw_market::Kind::decl(cfg),
        vw_market::Pairing::decl(cfg),
        vw_market::Pack::decl(cfg),
        vw_market::RepoSource::decl(cfg),
        vw_market::FileSpec::decl(cfg),
        vw_market::Edit::decl(cfg),
        vw_market::McpServerSpec::decl(cfg),
        vw_market::Server::decl(cfg),
        vw_market::Access::decl(cfg),
        vw_market::PluginSpec::decl(cfg),
        vw_market::Runtime::decl(cfg),
        vw_market::RuntimeTarget::decl(cfg),
        vw_market::Member::decl(cfg),
        vw_market::ArchiveFormat::decl(cfg),
        vw_market::NpmPackage::decl(cfg),
        vw_market::PostInstall::decl(cfg),
    ]
}

/// Libraries and image generation types, in their own module.
pub(super) fn work_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::LibrariesRequest::decl(cfg),
        crate::LibrariesResult::decl(cfg),
        crate::LibraryScope::decl(cfg),
        crate::LibraryFile::decl(cfg),
        crate::LibraryView::decl(cfg),
        crate::LibraryDocumentView::decl(cfg),
        crate::LibrarySourceView::decl(cfg),
        crate::LibraryUploadRefusal::decl(cfg),
        crate::WorkCosts::decl(cfg),
        crate::ImagesRequest::decl(cfg),
        crate::ImagesResult::decl(cfg),
    ]
}

/// Speech types, in their own module.
pub(super) fn speech_decls(cfg: &Config) -> Vec<String> {
    vec![
        SpeechRequest::decl(cfg),
        SpeechResult::decl(cfg),
        DictationMode::decl(cfg),
        DictationUpdate::decl(cfg),
        VoiceView::decl(cfg),
    ]
}

/// Worktree types, in their own module.
pub(super) fn worktrees_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::WorktreesRequest::decl(cfg),
        crate::WorktreesResult::decl(cfg),
        crate::WorktreeOptions::decl(cfg),
        crate::WorktreeInfo::decl(cfg),
        crate::WorktreeState::decl(cfg),
        crate::WorktreeInspection::decl(cfg),
        crate::BranchPlan::decl(cfg),
        crate::RemoveOutcome::decl(cfg),
        crate::OrphanWorktree::decl(cfg),
        vw_worktrees::Head::decl(cfg),
        vw_worktrees::BranchInfo::decl(cfg),
        vw_worktrees::LostFile::decl(cfg),
        vw_worktrees::LostKind::decl(cfg),
        vw_worktrees::Decision::decl(cfg),
        vw_worktrees::IncludeReport::decl(cfg),
        vw_worktrees::IncludeFile::decl(cfg),
        vw_worktrees::IncludeSkip::decl(cfg),
        vw_worktrees::SkipReason::decl(cfg),
    ]
}

/// Project secrets and safety reminders, in their own module.
pub(super) fn secrets_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::SecretsRequest::decl(cfg),
        crate::SecretsResult::decl(cfg),
        crate::PasswordOptions::decl(cfg),
        crate::ProjectSecretView::decl(cfg),
        crate::EntryPermissionView::decl(cfg),
        crate::secrets_view::ProjectPermissionView::decl(cfg),
        crate::ServerGrantView::decl(cfg),
        crate::LoginView::decl(cfg),
        crate::FillGrantView::decl(cfg),
        crate::LoginPassword::decl(cfg),
        crate::SecretUseView::decl(cfg),
        crate::SafetyRequest::decl(cfg),
        crate::SafetyResult::decl(cfg),
        crate::SafetyRuleView::decl(cfg),
        crate::CustomRuleView::decl(cfg),
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

/// Project git browser types, separate from the existing Git action contract.
pub(super) fn git_browser_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::ExplorerRequest::decl(cfg),
        crate::ExplorerResult::decl(cfg),
        crate::ExplorerEntry::decl(cfg),
        crate::ExplorerVersion::decl(cfg),
        crate::ExplorerHit::decl(cfg),
        crate::GitBrowserRequest::decl(cfg),
        crate::GitBrowserResult::decl(cfg),
        crate::GitBrowserSnapshot::decl(cfg),
        crate::GitBrowserNode::decl(cfg),
        vw_git::browser_status::BrowserStatus::decl(cfg),
        vw_git::browser_status::BrowserFile::decl(cfg),
    ]
    .into_iter()
    .map(|decl| trim_decl_lines(&decl))
    .collect()
}

/// The Git request is large because each operation documents its own selected repository.
pub(super) fn git_request_decls(cfg: &Config) -> Vec<String> {
    vec![trim_decl_lines(&crate::GitRequest::decl(cfg))]
}

pub(super) fn trim_decl_lines(decl: &str) -> String {
    decl.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Changes tab types, in their own module for the same size reason.
pub(super) fn changes_decls(cfg: &Config) -> Vec<String> {
    vec![
        trim_decl_lines(&ChangesRequest::decl(cfg)),
        trim_decl_lines(&ChangesResult::decl(cfg)),
        ChangesView::decl(cfg),
        ChangesMode::decl(cfg),
        TurnChanges::decl(cfg),
        ChangesSubagent::decl(cfg),
        TurnFile::decl(cfg),
        UndoPreview::decl(cfg),
        FolderSession::decl(cfg),
        vw_changes::Target::decl(cfg),
        vw_changes::FileChange::decl(cfg),
        vw_changes::FileStatus::decl(cfg),
        vw_changes::ContentKind::decl(cfg),
        vw_changes::Hunk::decl(cfg),
        vw_changes::DiffLine::decl(cfg),
        vw_changes::LineKind::decl(cfg),
        vw_changes::TurnLabel::decl(cfg),
        vw_changes::TurnSource::decl(cfg),
        vw_changes::InitPlan::decl(cfg),
        crate::LineCommentsRequest::decl(cfg),
        crate::LineCommentsResult::decl(cfg),
        crate::LineCommentState::decl(cfg),
        crate::LineCommentView::decl(cfg),
        vw_changes::LineComment::decl(cfg),
        vw_changes::LineAnchor::decl(cfg),
    ]
}

/// Key and vault types, in their own module for the same size reason.
pub(super) fn keys_decls(cfg: &Config) -> Vec<String> {
    vec![
        KeyRequest::decl(cfg),
        crate::MistralKeyView::decl(cfg),
        crate::MistralKeyTestView::decl(cfg),
        SecretInput::decl(cfg),
        KeyStatusView::decl(cfg),
        KeyMetadataView::decl(cfg),
        KeySourceView::decl(cfg),
        VibeKeyView::decl(cfg),
        VibeKeySource::decl(cfg),
        VaultChoice::decl(cfg),
        crate::VaultUnlockMethod::decl(cfg),
    ]
}

/// The Vibe install at setup, in its own module.
pub(super) fn vibe_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::VibeRequest::decl(cfg),
        vw_vibe_install::InstallView::decl(cfg),
        vw_vibe_install::InstallPhase::decl(cfg),
        vw_vibe_install::InstallOffer::decl(cfg),
        vw_vibe_install::SourceView::decl(cfg),
        vw_vibe_install::ExistingView::decl(cfg),
        vw_vibe_install::FailKind::decl(cfg),
        vw_vibe_install::InstallSource::decl(cfg),
        vw_vibe_install::VibeUpdateView::decl(cfg),
        vw_vibe_install::VibeUpdatePhase::decl(cfg),
    ]
}

/// Document types, in their own module.
pub(super) fn documents_decls(cfg: &Config) -> Vec<String> {
    vec![
        DocumentsRequest::decl(cfg),
        DocumentsResult::decl(cfg),
        DocumentView::decl(cfg),
        KeptView::decl(cfg),
        KeptUnitView::decl(cfg),
        DocumentRefusal::decl(cfg),
        DocumentRefusalKind::decl(cfg),
        DocumentSource::decl(cfg),
        crate::DroppedFile::decl(cfg),
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

/// Instruction file types, in their own module.
pub(super) fn instructions_decls(cfg: &Config) -> Vec<String> {
    vec![
        InstructionsRequest::decl(cfg),
        InstructionsResult::decl(cfg),
        InstructionsDoc::decl(cfg),
        InstructionsScope::decl(cfg),
        ProjectInstructionFile::decl(cfg),
        ProjectInstructionsRequest::decl(cfg),
        ProjectInstructionsResult::decl(cfg),
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

/// Terminal types, written to their own module so each generated file stays under the
/// 800-line file size limit.
pub(super) fn terminal_decls(cfg: &Config) -> Vec<String> {
    vec![
        TerminalRequest::decl(cfg),
        TerminalResult::decl(cfg),
        TerminalView::decl(cfg),
        TerminalStateView::decl(cfg),
        TerminalOutputView::decl(cfg),
        TerminalEventView::decl(cfg),
    ]
}

/// Status and diagnostics views (`readStatus`, `readAnomalies`, server events), in their own module
/// for the same reason.
pub(super) fn status_decls(cfg: &Config) -> Vec<String> {
    vec![
        AnomalyCountsView::decl(cfg),
        AnomalyRecordView::decl(cfg),
        AnomalyReport::decl(cfg),
        CoreStatus::decl(cfg),
        RateLimitsView::decl(cfg),
        ChannelsReport::decl(cfg),
        ChannelView::decl(cfg),
        StuckView::decl(cfg),
        ServerReport::decl(cfg),
        ServerStatus::decl(cfg),
        RecoveringView::decl(cfg),
        DeathView::decl(cfg),
        DebugApiFlag::decl(cfg),
        // small enough to share this module.
        crate::ProcessesRequest::decl(cfg),
        crate::ProcessesResult::decl(cfg),
        crate::LeftRunningView::decl(cfg),
        crate::SessionProcessView::decl(cfg),
    ]
}

/// The event stream's type (`CoreEvent`), in its own module so `index.ts` stays under the size limit (it
/// was at 793 of 800 lines).
pub(super) fn events_decls(cfg: &Config) -> Vec<String> {
    vec![
        CoreEvent::decl(cfg),
        SubagentCard::decl(cfg),
        crate::GuardKind::decl(cfg),
    ]
    .into_iter()
    .map(|decl| trim_decl_lines(&decl))
    .collect()
}

pub(super) fn core_decls(cfg: &Config) -> Vec<String> {
    vec![
        // ts-rs leaves a space before multiline variant comments; keep regenerated commands diff-clean.
        Command::decl(cfg)
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n"),
        CommandResult::decl(cfg)
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n"),
        CoreError::decl(cfg),
        ErrorCode::decl(cfg),
        Origin::decl(cfg),
        // Shared by skills.ts and market.ts (one-click install offers).
        crate::ToolsInstallOffer::decl(cfg),
        AlertKind::decl(cfg),
        CallbackInfo::decl(cfg),
    ]
}

/// The session list's view types (snapshot, row, change), in their own module so `index.ts` stays within the
/// 800-line size limit, alongside the session tools (export, temporary sessions, search).
pub(super) fn sessions_decls(cfg: &Config) -> Vec<String> {
    let mut decls = vec![
        SessionChange::decl(cfg),
        SessionSnapshot::decl(cfg),
        SessionRowView::decl(cfg),
    ];
    decls.extend(session_tools_decls(cfg));
    decls
}

pub(super) fn push_decls(out: &mut String, decls: &[String]) {
    for decl in decls {
        out.push('\n');
        out.push_str("export ");
        out.push_str(&trim_decl_lines(decl));
        out.push('\n');
    }
}

/// File tab types, in their own module (`Rect` from `preview.ts`, `ViewerView` from `pages.ts`).
pub(super) fn files_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::FilesRequest::decl(cfg),
        crate::FilesResult::decl(cfg),
        crate::FileRefView::decl(cfg),
        crate::FileChoice::decl(cfg),
        crate::FileContent::decl(cfg),
        crate::FileKind::decl(cfg),
        crate::PanelView::decl(cfg),
        crate::PanelEditor::decl(cfg),
        crate::PanelSelection::decl(cfg),
        crate::PanelFiles::decl(cfg),
    ]
}

/// Session tools types (export, temporary sessions, search) live in `sessions.ts` too (see
/// [`sessions_decls`]).
pub(super) fn session_tools_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::SessionToolsRequest::decl(cfg),
        crate::SessionToolsResult::decl(cfg),
        crate::SessionExportView::decl(cfg),
        crate::SessionExcerptView::decl(cfg),
        crate::SearchRequest::decl(cfg),
        crate::SearchResult::decl(cfg),
        crate::MessageHit::decl(cfg),
        crate::IndexStatus::decl(cfg),
        crate::FileHit::decl(cfg),
    ]
}
