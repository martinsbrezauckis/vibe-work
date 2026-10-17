//! TypeScript bindings of the core API for the UI (`ui/src/protocol/core/{index,sessions,status,terminal,keys,instructions,changes,documents,speech,preview,connectors,market,worktrees,files,work,events}.ts`), rendered by
//! `ts-rs` from the Rust types so the two can never drift (the drift check in
//! `scripts/public/check.sh` regenerates and compares).
//!
//! Depends on: `ts-rs` (feature `ts`), every public API type. Called by: the `gen-core-ts`
//! binary.
//! Invariant: every type that appears in a `Command`, `CommandResult`, `CoreEvent` or `CoreError`
//! is declared in the output; a missing one fails the UI typecheck.

use ts_rs::{Config, TS};

mod git;
use git::git_decls;

mod preview;
use preview::{browser_decls, preview_decls};

use crate::{
    AlertKind, AnomalyCountsView, AnomalyRecordView, AnomalyReport, CallbackInfo, ChannelView,
    ChannelsReport, Command, CommandResult, CoreError, CoreEvent, CoreStatus, DeathView,
    DebugApiFlag, DocumentRefusal, DocumentSource, DocumentView, DocumentsRequest, DocumentsResult,
    ErrorCode, InstructionsDoc, InstructionsRequest, InstructionsResult, InstructionsScope,
    KeptUnitView, KeptView, KeyCandidateView, KeyMetadataView, KeyRequest, KeySourceView,
    KeyStatusView, Origin, ProjectInstructionFile, ProjectInstructionsRequest,
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

/// Connector and MCP server types, in their own module.
fn connectors_decls(cfg: &Config) -> Vec<String> {
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

/// Settings > Network types, in their own module.
fn network_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::NetworkRequest::decl(cfg),
        crate::NetworkChanges::decl(cfg),
        crate::NetworkResult::decl(cfg),
        crate::NetworkView::decl(cfg),
        crate::NetworkFieldView::decl(cfg),
        crate::NetworkProblem::decl(cfg),
        crate::NetworkTestKind::decl(cfg),
        crate::NetworkTestView::decl(cfg),
    ]
}

/// Marketplace types, in their own module.
fn market_decls(cfg: &Config) -> Vec<String> {
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
fn work_decls(cfg: &Config) -> Vec<String> {
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
fn speech_decls(cfg: &Config) -> Vec<String> {
    vec![
        SpeechRequest::decl(cfg),
        SpeechResult::decl(cfg),
        DictationMode::decl(cfg),
        DictationUpdate::decl(cfg),
        VoiceView::decl(cfg),
    ]
}
/// Worktree types, in their own module.
fn worktrees_decls(cfg: &Config) -> Vec<String> {
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

/// Project handbook types, in their own module for the same size reason (`Rect` comes from `preview.ts`).
fn pages_decls(cfg: &Config) -> Vec<String> {
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
        vw_pages::LegacyHead::decl(cfg),
        vw_pages::Step::decl(cfg),
        vw_pages::Section::decl(cfg),
        vw_pages::Comment::decl(cfg),
        vw_pages::PagesSettings::decl(cfg),
        vw_pages::PickedSection::decl(cfg),
        vw_pages::Picked::decl(cfg),
        vw_pages::ViewerView::decl(cfg),
    ]
}

/// Handbook types in their own module so `pages.ts` stays under the size limit (`pages.ts` imports them): the
/// Projects overview's heads and Format 1.2's page inputs, answers and visual check.
fn handbook_decls(cfg: &Config) -> Vec<String> {
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
fn projects_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::ProjectsRequest::decl(cfg),
        crate::ProjectsResult::decl(cfg),
        crate::ProjectView::decl(cfg),
    ]
}

/// The outer git loop, in its own module for the same size reason.
/// The submodules that also import `SecretInput` from `keys.ts`: `secrets.ts` and `connectors.ts`
/// (a key server's new secret).
fn keys_importing_modules(
    header: &str,
    secrets: &[String],
    core: &[String],
    cfg: &Config,
) -> [(String, String); 2] {
    let keys = keys_decls(cfg);
    let connectors = connectors_decls(cfg);
    let module = |decls: &[String], file: &str| {
        (
            format!("{file}.ts"),
            add_import(&submodule(header, decls, core), decls, &keys, "keys"),
        )
    };
    [
        module(secrets, "secrets"),
        module(&connectors, "connectors"),
    ]
}

/// Project secrets and safety reminders, in their own module.
fn secrets_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::SecretsRequest::decl(cfg),
        crate::SecretsResult::decl(cfg),
        crate::PasswordOptions::decl(cfg),
        crate::ProjectSecretView::decl(cfg),
        crate::EntryPermissionView::decl(cfg),
        crate::secrets_view::ProjectPermissionView::decl(cfg),
        crate::GrantView::decl(cfg),
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
fn git_browser_decls(cfg: &Config) -> Vec<String> {
    vec![
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
fn git_request_decls(cfg: &Config) -> Vec<String> {
    vec![trim_decl_lines(&crate::GitRequest::decl(cfg))]
}

fn trim_decl_lines(decl: &str) -> String {
    decl.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Changes tab types, in their own module for the same size reason.
fn changes_decls(cfg: &Config) -> Vec<String> {
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
fn keys_decls(cfg: &Config) -> Vec<String> {
    vec![
        KeyRequest::decl(cfg),
        SecretInput::decl(cfg),
        KeyStatusView::decl(cfg),
        KeyMetadataView::decl(cfg),
        KeySourceView::decl(cfg),
        KeyCandidateView::decl(cfg),
        VibeKeyView::decl(cfg),
        VibeKeySource::decl(cfg),
        VaultChoice::decl(cfg),
    ]
}

/// The Vibe install at setup, in its own module.
fn vibe_decls(cfg: &Config) -> Vec<String> {
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
fn documents_decls(cfg: &Config) -> Vec<String> {
    vec![
        DocumentsRequest::decl(cfg),
        DocumentsResult::decl(cfg),
        DocumentView::decl(cfg),
        KeptView::decl(cfg),
        KeptUnitView::decl(cfg),
        DocumentRefusal::decl(cfg),
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
fn instructions_decls(cfg: &Config) -> Vec<String> {
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
fn terminal_decls(cfg: &Config) -> Vec<String> {
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
fn status_decls(cfg: &Config) -> Vec<String> {
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
fn events_decls(cfg: &Config) -> Vec<String> {
    vec![CoreEvent::decl(cfg), SubagentCard::decl(cfg)]
        .into_iter()
        .map(|decl| trim_decl_lines(&decl))
        .collect()
}

fn core_decls(cfg: &Config) -> Vec<String> {
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
        AlertKind::decl(cfg),
        CallbackInfo::decl(cfg),
    ]
}

/// The session list's view types (snapshot, row, change), in their own module so `index.ts` stays within the
/// 800-line size limit, alongside the session tools (export, temporary sessions, search).
fn sessions_decls(cfg: &Config) -> Vec<String> {
    let mut decls = vec![
        SessionChange::decl(cfg),
        SessionSnapshot::decl(cfg),
        SessionRowView::decl(cfg),
    ];
    decls.extend(session_tools_decls(cfg));
    decls
}

fn push_decls(out: &mut String, decls: &[String]) {
    for decl in decls {
        out.push('\n');
        out.push_str("export ");
        out.push_str(decl);
        out.push('\n');
    }
}

/// The type name a `decl` declares (`type Name = ...`).
fn declared_name(decl: &str) -> &str {
    decl.trim_start_matches("type ")
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
}

/// True when `text` mentions `name` as a whole word.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !word(before) && !word(after)
    })
}

/// `text` without its `/* ... */` comments (the generated doc comments).
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `import type { ... } from "./<module>";` for the names of `decls` that `body` uses.
fn imports(body: &str, decls: &[String], module: &str) -> String {
    // names that only appear in doc comments are not imports (TypeScript refuses unused ones).
    let code = without_comments(body);
    let used: Vec<&str> = decls
        .iter()
        .map(|d| declared_name(d))
        .filter(|name| mentions(&code, name))
        .collect();
    if used.is_empty() {
        String::new()
    } else {
        format!(
            "import type {{ {} }} from \"./{module}\";\n",
            used.join(", ")
        )
    }
}

/// A submodule that also uses types of one sibling module (`pages.ts` uses `Rect` from `preview.ts`; `preview.ts`
/// uses `Picked` from `pages.ts`).
fn submodule_with(
    header: &str,
    decls: &[String],
    core: &[String],
    sibling: &[String],
    name: &str,
) -> String {
    add_import(&submodule(header, decls, core), decls, sibling, name)
}

/// `text` (a submodule built from `decls`) with an import of what it uses from the sibling module `name`.
fn add_import(text: &str, decls: &[String], sibling: &[String], name: &str) -> String {
    let mut body = String::new();
    push_decls(&mut body, decls);
    let import = imports(&body, sibling, name);
    if import.is_empty() {
        return text.to_owned();
    }
    match text.find("\nexport ") {
        Some(at) => format!("{}{import}{}", &text[..=at], &text[at + 1..]),
        None => format!("{text}{import}"),
    }
}

/// File tab types, in their own module (`Rect` from `preview.ts`, `ViewerView` from `pages.ts`).
fn files_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::FilesRequest::decl(cfg),
        crate::FilesResult::decl(cfg),
        crate::FileRefView::decl(cfg),
        crate::FileChoice::decl(cfg),
        crate::FileContent::decl(cfg),
        crate::FileKind::decl(cfg),
    ]
}

/// Session tools types (export, temporary sessions, search) live in `sessions.ts` too (see
/// [`sessions_decls`]).
fn session_tools_decls(cfg: &Config) -> Vec<String> {
    vec![
        crate::SessionToolsRequest::decl(cfg),
        crate::SessionToolsResult::decl(cfg),
        crate::SessionExportView::decl(cfg),
        crate::SearchRequest::decl(cfg),
        crate::SearchResult::decl(cfg),
        crate::MessageHit::decl(cfg),
        crate::IndexStatus::decl(cfg),
        crate::FileHit::decl(cfg),
    ]
}

/// A submodule: header, its imports from `index` (`JsonValue`, core types), its declarations.
fn submodule(header: &str, decls: &[String], core: &[String]) -> String {
    let mut body = String::new();
    push_decls(&mut body, decls);
    let mut from_index = core.to_vec();
    // `JsonValue` is declared by hand in index.ts; a stand-in declaration lets `imports` find it.
    from_index.push("type JsonValue = never".to_owned());
    let mut out = String::from(header);
    let import = imports(&body, &from_index, "index");
    if !import.is_empty() {
        out.push('\n');
        out.push_str(&import);
    }
    out.push_str(&body);
    out
}

/// `events.ts` (`CoreEvent`): it uses types of many modules, one import line per sibling module it uses.
fn events_module(
    header: &str,
    events: &[String],
    core: &[String],
    siblings: &[(&[String], &str)],
) -> String {
    siblings
        .iter()
        .filter(|(_, module)| *module != "events")
        .fold(submodule(header, events, core), |text, (decls, module)| {
            add_import(&text, events, decls, module)
        })
}

/// `index.ts`: the header, `JsonValue`, an import of what the core declarations use from each sibling module, a
/// re-export of every sibling module, then the core declarations.
fn index_module(header: &str, core: &[String], siblings: &[(&[String], &str)]) -> String {
    use std::fmt::Write as _;
    let mut body = String::new();
    push_decls(&mut body, core);
    let body = body.as_str();
    let mut index = String::from(header);
    index.push_str(
        "\n/** Any JSON value (Vibe structures the core passes through untyped). */\n\
         export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]?: JsonValue } | null;\n\n",
    );
    for (decls, module) in siblings {
        index.push_str(&imports(body, decls, module));
    }
    for (_, module) in siblings {
        let _ = writeln!(index, "export type * from \"./{module}\";");
    }
    index.push_str(body);
    index
}
/// The generated modules as `(file name, text)`: `index.ts` (the command API, `JsonValue`, and
/// re-exports of the other modules), `status.ts` (status and diagnostics views) and `terminal.ts`
/// (terminal types), `keys.ts`, `vibe.ts`, `instructions.ts`, `changes.ts`, `documents.ts`,
/// `speech.ts`, `preview.ts`, `projects.ts`, `files.ts`, `handbook.ts`, `git.ts`, `network.ts`,
/// `git_request.ts`, `market.ts`, `work.ts`, `sessions.ts` (session list and tools),
/// `events.ts` (`CoreEvent`), and `secrets.ts` (project secrets and safety reminders).
/// Each file stays under the 800-line limit.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn typescript_modules(header: &str) -> Vec<(String, String)> {
    let cfg = Config::default();
    let core = core_decls(&cfg);
    let sessions = sessions_decls(&cfg);
    let status = status_decls(&cfg);
    let changes = changes_decls(&cfg);
    let preview = preview_decls(&cfg);
    let browser = browser_decls(&cfg);
    let pages = pages_decls(&cfg);
    let handbook = handbook_decls(&cfg);
    let worktrees = worktrees_decls(&cfg);
    let files = files_decls(&cfg);
    let git_request = git_request_decls(&cfg);
    let git = git_decls(&cfg);
    let events = events_decls(&cfg);
    let secrets = secrets_decls(&cfg);
    // Plain modules: only `index` imports from them (their declarations are used nowhere else).
    let plain = [
        (terminal_decls(&cfg), "terminal"),
        (keys_decls(&cfg), "keys"),
        (instructions_decls(&cfg), "instructions"),
        (documents_decls(&cfg), "documents"),
        (speech_decls(&cfg), "speech"),
        (connectors_decls(&cfg), "connectors"),
        (market_decls(&cfg), "market"),
        (network_decls(&cfg), "network"),
        (projects_decls(&cfg), "projects"),
        (vibe_decls(&cfg), "vibe"),
        (git.clone(), "git"),
        (git_browser_decls(&cfg), "git_browser"),
        (work_decls(&cfg), "work"),
        (
            vec![
                crate::SecretScanSettings::decl(&cfg),
                crate::SecretScanRequest::decl(&cfg),
                crate::SecretScanStatus::decl(&cfg),
                crate::SecretLeakName::decl(&cfg),
                crate::SecretLeakState::decl(&cfg),
                crate::SecretLeakFinding::decl(&cfg),
                crate::SecretScanView::decl(&cfg),
            ]
            .into_iter()
            .map(|decl| trim_decl_lines(&decl))
            .collect(),
            "secret_scan",
        ),
    ];
    let mut siblings: Vec<(&[String], &str)> = vec![
        (&sessions, "sessions"),
        (&status, "status"),
        (&changes, "changes"),
        (&preview, "preview"),
        (&browser, "browser"),
        (&pages, "pages"),
        (&handbook, "handbook"),
        (&worktrees, "worktrees"),
        (&files, "files"),
        (&events, "events"),
        (&secrets, "secrets"),
        (&git_request, "git_request"),
    ];
    siblings.extend(
        plain
            .iter()
            .map(|(decls, module)| (decls.as_slice(), *module)),
    );
    // Modules that also import from a sibling module are built by hand; every other sibling is a plain submodule
    // that imports only from `index`, keeping this function within clippy's line limit as modules are added.
    let mut modules = vec![
        (
            "changes.ts".to_owned(),
            submodule_with(header, &changes, &core, &git, "git"),
        ),
        (
            "index.ts".to_owned(),
            index_module(header, &core, &siblings),
        ),
        (
            "sessions.ts".to_owned(),
            submodule_with(header, &sessions, &core, &status, "status"),
        ),
        (
            "preview.ts".to_owned(),
            submodule_with(header, &preview, &core, &pages, "pages"),
        ),
        (
            "browser.ts".to_owned(),
            submodule_with(header, &browser, &core, &preview, "preview"),
        ),
        (
            "pages.ts".to_owned(),
            add_import(
                &submodule_with(header, &pages, &core, &preview, "preview"),
                &pages,
                &handbook,
                "handbook",
            ),
        ),
        (
            "handbook.ts".to_owned(),
            submodule_with(header, &handbook, &core, &pages, "pages"),
        ),
        (
            "worktrees.ts".to_owned(),
            add_import(
                &submodule_with(header, &worktrees, &core, &changes, "changes"),
                &worktrees,
                &sessions,
                "sessions",
            ),
        ),
        (
            "files.ts".to_owned(),
            add_import(
                &submodule_with(header, &files, &core, &pages, "pages"),
                &files,
                &preview,
                "preview",
            ),
        ),
        (
            "events.ts".to_owned(),
            events_module(header, &events, &core, &siblings),
        ),
        (
            "git_request.ts".to_owned(),
            submodule_with(header, &git_request, &core, &git, "git"),
        ),
        (
            "git_browser.ts".to_owned(),
            submodule_with(header, &git_browser_decls(&cfg), &core, &git, "git"),
        ),
    ];
    modules.extend(keys_importing_modules(header, &secrets, &core, &cfg));
    for (decls, module) in &siblings {
        let name = format!("{module}.ts");
        if modules.iter().all(|(built, _)| *built != name) {
            modules.push((name, submodule(header, decls, &core)));
        }
    }
    modules
}
