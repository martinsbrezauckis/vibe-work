//! The typed requests both clients of the core send (the UI over Tauri IPC and the Debug
//! API over HTTP), who sent them, and what comes back.
//!
//! Depends on: `view` (result payloads), `serde`, `serde_json`. Called by: `dispatch` (executes
//! them), vw-debug-api (decodes them from request bodies), the UI through the generated TypeScript.
//! Invariants:
//! - Wire form: `{"type": "<camelCaseVariant>", ...camelCase fields}`; unknown fields are refused
//!   (`deny_unknown_fields`), so a misspelled key is an error and never a silent default.
//!   One identifier has one wire spelling across clients; aliases cannot silently diverge.
//! - Nested Vibe structures (`agentConfig`, message content, callback answers) stay JSON values
//!   here; the gate validates them against the generated protocol types before anything is sent.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::changes_view::{ChangesRequest, ChangesResult};
use crate::connectors_view::{ConnectorsRequest, ConnectorsResult};
use crate::documents_view::{DocumentsRequest, DocumentsResult};
use crate::files_view::{FilesRequest, FilesResult};
use crate::git_view::{GitRequest, GitResult};
use crate::instructions_view::{InstructionsRequest, InstructionsResult};
use crate::keys::{KeyCandidateView, KeyRequest, KeyStatusView, VibeKeyView};
use crate::left_running_view::{ProcessesRequest, ProcessesResult};
use crate::line_comments_view::{LineCommentsRequest, LineCommentsResult};
use crate::market_view::{MarketRequest, MarketResult};
use crate::pages_view::{PagesRequest, PagesResult};
use crate::preview_view::{PreviewRequest, PreviewResult};
use crate::project_instructions_view::{ProjectInstructionsRequest, ProjectInstructionsResult};
use crate::projects_view::{ProjectsRequest, ProjectsResult};
use crate::safety_view::{SafetyRequest, SafetyResult};
use crate::search_view::{SearchRequest, SearchResult};
use crate::secrets_view::{SecretsRequest, SecretsResult};
use crate::session_tools_view::{SessionToolsRequest, SessionToolsResult};
use crate::speech_view::{SpeechRequest, SpeechResult};
use crate::terminal_view::{TerminalRequest, TerminalResult};
use crate::view::{AnomalyReport, CallbackInfo, CoreStatus, SessionRowView, SessionSnapshot};
use crate::work_view::{ImagesRequest, ImagesResult, LibrariesRequest, LibrariesResult};
use crate::worktrees_view::{WorktreesRequest, WorktreesResult};

/// Which client sent a command. Logged with every refusal so later intent rules can differ per
/// client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    /// The app's own webview (Tauri bridge).
    Ui,
    /// The loopback Debug API (an external agent or a test).
    DebugApi,
}

/// The dialog's requester for a command's origin, kept as a structured field.
#[must_use]
pub(crate) fn requester(origin: Origin) -> vw_gate::Requester {
    match origin {
        Origin::Ui => vw_gate::Requester::Window,
        Origin::DebugApi => vw_gate::Requester::DebugApi,
    }
}

impl Origin {
    /// The wire name (`ui`, `debug-api`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::DebugApi => "debug-api",
        }
    }
}

/// One request to the core. Every variant that reaches Vibe is executed through the method gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    /// `session/list` merged with sessions the server does not list yet.
    ListSessions {
        /// Page size (server default when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        limit: Option<u32>,
        /// Cursor from a previous page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cursor: Option<String>,
        /// Also list archived sessions (Vibe's `includeArchived`; default false).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        include_archived: Option<bool>,
    },
    /// The store's copy of one loaded session (no server call).
    ReadSession {
        /// Session id.
        session_id: String,
    },
    /// `session/read`, merged into the store (the latest page wins).
    RefreshSession {
        /// Session id.
        session_id: String,
    },
    /// `session/start`; the new session is loaded and tracked for restarts.
    StartSession {
        /// `AgentConfig` object (filtered by the gate).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_config: Option<Value>,
    },
    /// `session/resume` (after a restart or to reopen a stored session).
    ResumeSession {
        /// Session id.
        session_id: String,
        /// `AgentConfig` object (filtered by the gate).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_config: Option<Value>,
    },
    /// `session/fork`: a new session from a source session, optionally cut at an entry or turn.
    ForkSession {
        /// The session to fork.
        source_session_id: String,
        /// Anchor entry (a `clientUserMessageId` the gate set).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        entry_id: Option<String>,
        /// Fork after this turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        after_turn_id: Option<String>,
        /// `AgentConfig` object (filtered by the gate).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        agent_config: Option<Value>,
    },
    /// `session/rewind` to an entry, in place or as a new session.
    RewindSession {
        /// Session id.
        session_id: String,
        /// Anchor entry.
        entry_id: String,
        /// Restore files changed after the anchor.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        restore_files: Option<bool>,
        /// Rewind this session instead of creating a new one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        inplace: Option<bool>,
    },
    /// `turn/start` (the gate guarantees a `clientUserMessageId`).
    StartTurn {
        /// Session id.
        session_id: String,
        /// User content blocks (`ContentBlock[]`).
        message: Vec<Value>,
        /// Anchor id to use; the gate makes one when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        client_user_message_id: Option<String>,
    },
    /// `turn/steer`: add a message to the running turn.
    SteerTurn {
        /// Session id.
        session_id: String,
        /// The running turn.
        expected_turn_id: String,
        /// User content blocks.
        message: Vec<Value>,
        /// Anchor id to use; the gate makes one when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        client_user_message_id: Option<String>,
    },
    /// `turn/interrupt`.
    InterruptTurn {
        /// Session id.
        session_id: String,
        /// The running turn.
        expected_turn_id: String,
    },
    /// Stops a named session's compaction by closing its channel; the next send reopens it.
    StopSession {
        /// Session whose current work is cancelled, preserving its saved conversation.
        session_id: String,
    },
    /// `config/model/write`: pin a model and/or thinking level to a session.
    PickModel {
        /// Session id.
        session_id: String,
        /// A configured model alias.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        model_alias: Option<String>,
        /// `off`, `low`, `medium`, `high` or `max`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reasoning_effort: Option<String>,
    },
    /// Save the model for future sessions in Vibe Work's own Vibe home, without attaching a session.
    SetDefaultModel {
        /// An alias offered by the current lobby config.
        model_alias: String,
    },
    /// `session/agent/update`: switch the access mode (one of Vibe's profiles). The core supplies
    /// the token for the trusted window's typed switch; other Full access callers need consent.
    SetAccessMode {
        /// Session id.
        session_id: String,
        /// Profile name.
        agent_name: String,
        /// Token from `requestConfirmation`, when the switch needs one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        confirmation_token: Option<String>,
    },
    /// Answer an open server callback (`callback/result`, validated by the gate).
    AnswerCallback {
        /// Session the callback belongs to.
        session_id: String,
        /// Callback id.
        callback_id: String,
        /// The `output` object, e.g. `{type: "approval", decision: {type: "approve"}}`.
        output: Value,
    },
    /// Callbacks waiting for an answer.
    ListCallbacks {},
    /// Show the native dialog for one exact call and return its single-use token. The Debug API
    /// normally acts as the user, but Full access still asks in the native dialog.
    RequestConfirmation {
        /// Method to confirm.
        method: String,
        /// Its exact params.
        params: Value,
    },
    /// Any other client method, through the gate's table (default deny).
    Call {
        /// JSON-RPC method.
        method: String,
        /// Its params object.
        params: Value,
        /// Token from `requestConfirmation`, when the call needs one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        confirmation_token: Option<String>,
    },
    /// Tells the core which session this client's main view shows (`None`: none). A viewed session
    /// keeps its channel open however long it is idle (`CoreConfig::channel_idle_close`).
    /// No server call.
    ViewSession {
        /// The shown session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// One attempt to reopen a stuck session (`session/resume` + profile restore), bounded by
    /// `CoreConfig::retry_reopen_timeout`; progress is in the server status' `recovering` list.
    /// No automatic retries exist: this is the only way a stuck session is tried again.
    RetryReopen {
        /// The stuck session.
        session_id: String,
    },
    /// Stops waiting for a running `retryReopen` of the session (it then fails with `cancelled`).
    CancelReopen {
        /// The session.
        session_id: String,
    },
    /// Starts a new session in a stuck session's folder with its profile, and links the two.
    ContinueInNewSession {
        /// The stuck session.
        session_id: String,
    },
    /// Supervisor state, versions, uptime and the Debug API flag (no server call).
    ReadStatus {},
    /// One session's anomaly counters and recent records (no server call).
    ReadAnomalies {
        /// Session id.
        session_id: String,
    },
    /// A key or vault operation (`keys.rs`): handled by the core's key service, never
    /// sent to Vibe, never answered with a secret.
    Keys {
        /// The operation.
        request: KeyRequest,
    },
    /// The Vibe install at setup (`vibe_install.rs`): offer, consent, progress, an existing
    /// install. Answers the install view.
    Vibe {
        /// The operation.
        request: crate::vibe_install::VibeRequest,
    },
    /// Integrated terminals: open, list, write, resize, read, restart, close. No server
    /// call.
    Terminal {
        /// What to do.
        request: TerminalRequest,
    },
    /// The `AGENTS.md` instruction files: read, read the CLI copy, write. No server
    /// call.
    Instructions {
        /// What to do.
        request: InstructionsRequest,
    },
    /// User-managed instruction files in a saved project's main folder.
    ProjectInstructions {
        /// The operation.
        request: ProjectInstructionsRequest,
    },
    /// Documents the user added through the native picker or an OS drop: list, read,
    /// extract with Mistral OCR, remove, take a picture's bytes. No server call; no operation adds a
    /// file.
    Documents {
        /// What to do.
        request: DocumentsRequest,
    },
    /// Voice: dictation streams (start, append, stop, cancel), text-to-speech, the voices
    /// list. Mistral is called by the core with the key from the vault.
    Speech {
        /// What to do.
        request: SpeechRequest,
    },
    /// Mistral document Libraries: list, create, scope, delete, documents, upload bytes, delete a
    /// document, read its text, ask across libraries. Mistral is called by the core with the key from the vault.
    Libraries {
        /// What to do.
        request: LibrariesRequest,
    },
    /// Image generation: use facts and the request count.
    Images {
        /// What to do.
        request: ImagesRequest,
    },
    /// The Changes tab: read, keep, revert, undo preview, folder sessions, version history. Git
    /// work in the core; `previewUndo` reads `session/rewind/read` through the gate.
    Changes {
        /// What to do.
        request: ChangesRequest,
    },
    /// Comments on lines of the Changes diff: list, add, remove, mark sent. Stored in app data.
    LineComments {
        /// What to do.
        request: LineCommentsRequest,
    },
    /// The preview pane: list, open, history, width, layout, close, screenshot, console, busy-port
    /// explanations. No server call.
    Preview {
        /// What to do.
        request: PreviewRequest,
    },
    /// The agent browser: read, act, the agent strip's state, Stop, Resume, annotations. No server call.
    Browser {
        /// What to do.
        request: crate::browser_view::BrowserRequest,
    },
    /// Project Pages: hub, pages, the viewer, section picks, comments, settings. No server call.
    Pages {
        /// What to do.
        request: PagesRequest,
    },
    /// The user's project groups: list, refresh, create, rename, delete (sessions move to "not in a
    /// project", none is deleted), move sessions, place, main folder, and a session start filed into a project.
    /// No server call except the reconcile's `session/list` and the start's `session/start` (through the gate).
    /// Both clients act alike; the Debug API does not require a native dialog.
    Projects {
        /// What to do.
        request: ProjectsRequest,
    },
    /// The File tab: resolve paths from the chat, read a file, show an HTML page or PDF in the isolated
    /// viewer, open with the default app (never a program), show in folder. No server call; the Debug API acts as
    /// the user (no dialogs); the window's open of an HTML/SVG file asks first.
    Files {
        /// What to do.
        request: FilesRequest,
    },
    /// Connectors and MCP servers: the catalog, Mistral sign-ins, servers added by address
    /// and their sign-ins. Adding a server asks the native dialog from the window (the Debug API acts as the
    /// user and is not asked).
    Connectors {
        /// What to do.
        request: ConnectorsRequest,
    },
    /// The Tools marketplace: the curated catalog of skill packs, MCP servers and plugins with what is
    /// installed; install (native dialog from the window; the Debug API acts as the user), remove.
    Marketplace {
        /// What to do.
        request: MarketRequest,
    },
    /// Worktree per session: options for a new session, the suggested branch, a start in a new
    /// copy, a session's copy, inspection and removal at the end, orphans. `remove` asks the window's native
    /// dialog, naming the files (the Debug API is not asked).
    Worktrees {
        /// What to do.
        request: WorktreesRequest,
    },
    /// The outer git loop of the Changes tab: the branch bar, Commit, Push, pull requests, the PR
    /// card, "Bring into main", per-project settings. Push, a PR made with gh and Bring in ask the window's native
    /// dialog naming what happens (the Debug API is not asked).
    Git {
        /// What to do.
        request: GitRequest,
    },
    /// Processes a deleted session left running: the list, the window's question right
    /// after a delete ("Also stop its N running processes?", native dialog; the Debug API is not asked and gets the
    /// list), and Stop for one of them. No server call.
    Processes {
        /// What to do.
        request: ProcessesRequest,
    },
    /// Project secrets: the secrets agents may name, add / replace / remove, and the "always" grants
    /// (list, revoke). No dialog (the user's own secrets; the Debug API acts as the user); no answer carries a
    /// value.
    Secrets {
        /// What to do.
        request: SecretsRequest,
    },
    /// Safety reminders: the rules the agent's commands are checked against, switched on
    /// or off, and the user's own rules.
    Safety {
        /// What to do.
        request: SafetyRequest,
    },
    /// The Debug API's "Choose folder": records `path` as a folder the user chose, like a pick in
    /// the native dialog (only such folders can be trusted, hold extra workspace roots or get a worktree copy).
    /// Debug API origin only: the window picks folders in the native dialog (its webview is the untrusted side
    /// for the gate's folder rules), so from the window this is `denied`.
    PickFolder {
        /// An existing folder (absolute path).
        path: String,
    },
    /// Session tools: export a session as Markdown, the temporary sessions. No server call but
    /// `session/history/list` and `session/read`.
    SessionTools {
        /// What to do.
        request: SessionToolsRequest,
    },
    /// Search inside messages and files: every conversation's messages from the core's index, file names
    /// and lines in the sessions' folders. No server call but `session/list` and `session/history/list`.
    Search {
        /// What to search.
        request: SearchRequest,
    },
}

impl Command {
    /// The variant name as on the wire (`startTurn`, ...), for logs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ListSessions { .. } => "listSessions",
            Self::ReadSession { .. } => "readSession",
            Self::RefreshSession { .. } => "refreshSession",
            Self::StartSession { .. } => "startSession",
            Self::ResumeSession { .. } => "resumeSession",
            Self::ForkSession { .. } => "forkSession",
            Self::RewindSession { .. } => "rewindSession",
            Self::StartTurn { .. } => "startTurn",
            Self::SteerTurn { .. } => "steerTurn",
            Self::InterruptTurn { .. } => "interruptTurn",
            Self::StopSession { .. } => "stopSession",
            Self::PickModel { .. } => "pickModel",
            Self::SetDefaultModel { .. } => "setDefaultModel",
            Self::SetAccessMode { .. } => "setAccessMode",
            Self::AnswerCallback { .. } => "answerCallback",
            Self::ListCallbacks {} => "listCallbacks",
            Self::RequestConfirmation { .. } => "requestConfirmation",
            Self::Call { .. } => "call",
            Self::ViewSession { .. } => "viewSession",
            Self::RetryReopen { .. } => "retryReopen",
            Self::CancelReopen { .. } => "cancelReopen",
            Self::ContinueInNewSession { .. } => "continueInNewSession",
            Self::ReadStatus {} => "readStatus",
            Self::ReadAnomalies { .. } => "readAnomalies",
            Self::Keys { .. } => "keys",
            Self::Vibe { .. } => "vibe",
            Self::Terminal { .. } => "terminal",
            Self::Instructions { .. } => "instructions",
            Self::ProjectInstructions { .. } => "projectInstructions",
            Self::Documents { .. } => "documents",
            Self::Speech { .. } => "speech",
            Self::Libraries { .. } => "libraries",
            Self::Images { .. } => "images",
            Self::Changes { .. } => "changes",
            Self::LineComments { .. } => "lineComments",
            Self::Preview { .. } => "preview",
            Self::Browser { .. } => "browser",
            Self::Pages { .. } => "pages",
            Self::Projects { .. } => "projects",
            Self::Files { .. } => "files",
            Self::Connectors { .. } => "connectors",
            Self::Marketplace { .. } => "marketplace",
            Self::Worktrees { .. } => "worktrees",
            Self::Git { .. } => "git",
            Self::Processes { .. } => "processes",
            Self::Secrets { .. } => "secrets",
            Self::Safety { .. } => "safety",
            Self::PickFolder { .. } => "pickFolder",
            Self::SessionTools { .. } => "sessionTools",
            Self::Search { .. } => "search",
        }
    }
}

/// What a successful command returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandResult {
    /// `listSessions`.
    Sessions {
        /// Local-only sessions first (newest first), then the server's page in order.
        rows: Vec<SessionRowView>,
        /// Cursor for the next page, when there is one.
        next_cursor: Option<String>,
        /// The session `vibe --continue` would resume (the response's `continueSessionId`).
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
    /// A one-way key or vault operation succeeded (nothing else is returned).
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
    /// `keys` / `listKeyCandidates`.
    KeyCandidates {
        /// Mistral credentials already in the vault (metadata).
        candidates: Vec<KeyCandidateView>,
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
    /// `marketplace`.
    Marketplace {
        /// The marketplace request's answer.
        result: MarketResult,
    },
    /// `worktrees`.
    Worktrees {
        /// The worktree request's answer.
        result: WorktreesResult,
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
    /// `safety`.
    Safety {
        /// The safety settings after the request.
        result: SafetyResult,
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
