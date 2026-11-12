//! Typed command replies shared by the Tauri and Debug API clients.
//! Depends on area-specific views and serde; returned by dispatch and re-exported with `Command`.
//! Wire tags and fields retain the camelCase contract used by generated TypeScript.

use crate::changes_view::ChangesResult;
use crate::connectors_view::ConnectorsResult;
use crate::documents_view::DocumentsResult;
use crate::files_view::FilesResult;
use crate::git_browser_view::GitBrowserResult;
use crate::git_view::GitResult;
use crate::instructions_view::InstructionsResult;
use crate::keys::{KeyStatusView, VibeKeyView};
use crate::left_running_view::ProcessesResult;
use crate::line_comments_view::LineCommentsResult;
use crate::market_view::MarketResult;
use crate::network_view::NetworkResult;
use crate::pages_view::PagesResult;
use crate::preview_view::PreviewResult;
use crate::project_instructions_view::ProjectInstructionsResult;
use crate::projects_view::ProjectsResult;
use crate::safety_view::SafetyResult;
use crate::search_view::SearchResult;
use crate::secrets_view::SecretsResult;
use crate::session_tools_view::SessionToolsResult;
use crate::skills_view::SkillsResult;
use crate::speech_view::SpeechResult;
use crate::terminal_view::TerminalResult;
use crate::view::{AnomalyReport, CallbackInfo, CoreStatus, SessionRowView, SessionSnapshot};
use crate::work_view::{ImagesResult, LibrariesResult};
use crate::worktrees_view::WorktreesResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a successful command returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandResult {
    /// The feedback panel after an operation.
    Feedback {
        /// Core-owned review and diagnostic facts.
        view: crate::FeedbackView,
    },
    /// Shared prompt history, oldest first (at most 100).
    PromptHistory {
        /// Submitted prompts, without known vault values.
        entries: Vec<String>,
    },
    /// The protected-folder decision and its readable record.
    ProtectedChange {
        /// Final review state.
        proposal: crate::ProtectedChangeView,
    },
    /// `listSessions`.
    Sessions {
        /// Local-only sessions first (newest first), then the server's page in order.
        rows: Vec<SessionRowView>,
        /// Cursor for the next page, when there is one.
        next_cursor: Option<String>,
        /// Retained for client compatibility; Vibe no longer supplies a CLI continuation target.
        continue_session_id: Option<String>,
    },
    /// Every command that loads or reads one session.
    Session {
        /// The store's copy after the command.
        snapshot: SessionSnapshot,
        /// The server's result without its `state` (e.g. rewind's `restoredPaths`); `null` for
        /// pure reads.
        detail: Value,
    },
    /// A forwarded call (turns, model pick, access mode, `call`, callback answers).
    Reply {
        /// The server's result.
        result: Value,
        /// For turn start/steer: the anchor id that was sent.
        client_user_message_id: Option<String>,
        /// For `session/turn/enqueue`: the `entryId` of each user entry.
        user_entry_ids: Vec<String>,
    },
    /// `listCallbacks`.
    Callbacks {
        /// Open callbacks, ordered by id.
        open: Vec<CallbackInfo>,
    },
    /// `requestConfirmation`: the user approved; pass the token with the call.
    Confirmation {
        /// Single-use token for exactly that method and params.
        token: String,
    },
    /// `readStatus`.
    Status {
        /// Supervisor, versions, uptime, Debug API flag.
        status: CoreStatus,
    },
    /// `readAnomalies`.
    Anomalies {
        /// Counters and recent records.
        report: AnomalyReport,
    },
    /// A one-way operation succeeded (nothing else is returned).
    Done {},
    /// Every `vibe` op: the install view after it ran.
    VibeInstall {
        /// Phase, offer, existing installs.
        view: vw_vibe_install::InstallView,
    },
    /// `keys` / `readStatus`.
    KeyStatus {
        /// Vault and key status (no secret).
        status: KeyStatusView,
    },
    /// All saved Mistral keys; metadata only.
    MistralKeys {
        /// Account rows without credential values.
        keys: Vec<crate::MistralKeyView>,
    },
    /// `keys` / `testKey`: the key works.
    KeyTest {
        /// Models the key can use.
        model_count: u32,
    },
    /// `keys` / `detectVibeKey`.
    VibeKeys {
        /// Where Vibe itself holds a key (no values).
        found: Vec<VibeKeyView>,
    },
    /// `terminal`.
    Terminal {
        /// The terminal request's answer.
        result: TerminalResult,
    },
    /// `instructions`.
    Instructions {
        /// The instructions request's answer.
        result: InstructionsResult,
    },
    /// `projectInstructions`.
    ProjectInstructions {
        /// The project instruction request's answer.
        result: ProjectInstructionsResult,
    },
    /// `documents`.
    Documents {
        /// The documents request's answer.
        result: DocumentsResult,
    },
    /// `changes`.
    Changes {
        /// The Changes request's answer.
        result: ChangesResult,
    },
    /// `lineComments`.
    LineComments {
        /// The line-comment request's answer.
        result: LineCommentsResult,
    },
    /// `preview`.
    Preview {
        /// The preview request's answer.
        result: PreviewResult,
    },
    /// `browser`.
    Browser {
        /// The browser request's answer.
        result: crate::browser_view::BrowserResult,
    },
    /// `pages`.
    Pages {
        /// The Pages request's answer.
        result: PagesResult,
    },
    /// `projects`.
    Projects {
        /// The project request's answer.
        result: ProjectsResult,
    },
    /// `files`.
    Files {
        /// The File tab request's answer.
        result: FilesResult,
    },
    /// `speech`.
    Speech {
        /// The speech request's answer.
        result: SpeechResult,
    },
    /// `libraries`.
    Libraries {
        /// The Libraries request's answer.
        result: LibrariesResult,
    },
    /// `images`.
    Images {
        /// The images request's answer.
        result: ImagesResult,
    },
    /// `connectors`.
    Connectors {
        /// The connectors request's answer.
        result: ConnectorsResult,
    },
    /// `skills`.
    Skills {
        /// The skills response.
        result: SkillsResult,
    },
    /// The reviewed Tools marketplace.
    Marketplace {
        /// The marketplace request's answer.
        result: MarketResult,
    },
    /// `worktrees`.
    Worktrees {
        /// The worktree request's answer.
        result: WorktreesResult,
    },
    /// `gitBrowser`.
    GitBrowser {
        /// Project browser answer.
        result: GitBrowserResult,
    },
    /// `git`.
    Git {
        /// The git request's answer.
        result: GitResult,
    },
    /// `processes`.
    Processes {
        /// The processes request's answer.
        result: ProcessesResult,
    },
    /// `secrets`.
    Secrets {
        /// The secrets request's answer.
        result: SecretsResult,
    },
    /// GitHub reader preference and account status.
    GithubReads {
        /// Current GitHub reader state.
        view: crate::GithubReadsView,
    },
    /// `askGrants`: every kept session grant, oldest first.
    AskGrants {
        /// The grants after the request.
        grants: Vec<vw_ask::SessionGrant>,
        /// What "Allow for this session" would keep on each open card that offers it (its covers line).
        offers: Vec<vw_ask::GrantOffer>,
        /// How many grants the request removed (0 for `list`).
        removed: u32,
    },
    /// `chromeControl`: Settings › Browser › Your Chrome after the request.
    ChromeControl {
        /// Availability, connection and chat work states.
        view: crate::ChromeControlView,
        /// The set up offer the window shows before the click (`offerSetup` only).
        offer: Option<Box<crate::ToolsInstallOffer>>,
    },
    /// Connected tabs for the composer picker.
    ChromeTabs {
        /// Tabs in all connected profiles.
        tabs: Vec<crate::ChromeTab>,
    },
    /// A selected Chrome context item claimed by a composer.
    ChromeSelection {
        /// Absent when another window already claimed it.
        selection: Option<crate::ChromeSelection>,
    },
    /// `vibeRules`: the lasting rules after the request.
    VibeRules {
        /// Each rule: a tool-wide `always`, or one added allowlist pattern.
        rules: Vec<crate::VibeRule>,
        /// Vibe's default allowlists were read, so its shipped patterns are left out.
        defaults_known: bool,
        /// How many rules the request removed (0 for `list`).
        removed: u32,
    },
    /// Automatic idle-sleep preference and status.
    KeepAwake {
        /// Core-owned view.
        view: crate::KeepAwakeView,
    },
    /// The New chat screen's owner id and folder.
    NewChat {
        /// The current state.
        view: crate::NewChatView,
    },
    /// Session log settings, status and retained findings.
    SecretScan {
        /// Value-free scanner view.
        view: crate::SecretScanView,
        /// File paths for Developer error detail only; absent from finding events and the bell.
        file_errors: Vec<String>,
    },
    /// `safety`.
    Safety {
        /// The safety settings after the request.
        result: SafetyResult,
    },
    /// `network`.
    Network {
        /// The network settings, a test result or the problems found.
        result: NetworkResult,
    },
    /// `sessionTools`.
    SessionTools {
        /// The session tools' answer.
        result: SessionToolsResult,
    },
    /// `search`.
    Search {
        /// The search's answer.
        result: SearchResult,
    },
}
