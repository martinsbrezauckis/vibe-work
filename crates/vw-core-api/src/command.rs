//! Typed core requests for Tauri UI and Debug API HTTP; executed by `dispatch`.
//! Depends on view payloads, serde and `serde_json`; the UI uses generated TypeScript bindings.
//! Wire tags and fields are camelCase, unknown fields are refused, and each identifier has one spelling.
//! Nested Vibe JSON is validated by the gate against generated protocol types before sending.
use crate::changes_view::ChangesRequest;
use crate::connectors_view::ConnectorsRequest;
use crate::documents_view::DocumentsRequest;
use crate::files_view::FilesRequest;
use crate::git_browser_view::GitBrowserRequest;
use crate::git_view::GitRequest;
use crate::instructions_view::InstructionsRequest;
use crate::keys::KeyRequest;
use crate::left_running_view::ProcessesRequest;
use crate::line_comments_view::LineCommentsRequest;
use crate::market_view::MarketRequest;
use crate::network_view::NetworkRequest;
use crate::pages_view::PagesRequest;
use crate::preview_view::PreviewRequest;
use crate::project_instructions_view::ProjectInstructionsRequest;
use crate::projects_view::ProjectsRequest;
use crate::safety_view::SafetyRequest;
use crate::search_view::SearchRequest;
use crate::secrets_view::SecretsRequest;
use crate::session_tools_view::SessionToolsRequest;
use crate::skills_view::SkillsRequest;
use crate::speech_view::SpeechRequest;
use crate::terminal_view::TerminalRequest;
use crate::work_view::{ImagesRequest, LibrariesRequest};
use crate::worktrees_view::WorktreesRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::command_origin::Origin;
pub(crate) use crate::command_origin::requester;
pub use crate::command_result::CommandResult;

/// One request to the core. External Vibe calls use the gate; plugin refresh owns its internal reload/reopen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    /// Feedback draft, review, consent and explicit delivery.
    Feedback {
        /// The panel operation.
        request: crate::FeedbackRequest,
    },
    /// Reads shared CLI prompt history, or appends one submitted prompt.
    PromptHistory {
        /// Absent reads; present appends in CLI format before reading.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        text: Option<String>,
    },
    /// Writes ordinary owner text to the native clipboard; no clipboard contents are read back.
    CopyText {
        /// The text to copy (at most one million bytes).
        text: String,
    },
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
    /// Re-pins this chat's plugins and reopens it with its host guards; working chats wait for turn end.
    RefreshPlugins {
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
        /// Provider identity from the selected row, retained while its availability is checked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        model_name: Option<String>,
        /// Display name from the selected row, for a notice if it has disappeared.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        model_display_name: Option<String>,
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
    /// The launch check of the default model: Mistral's own Large 4 entry replaces Vibe Work's once it is no longer
    /// marked preview (the default moves with it), and a default Vibe no longer offers is named so the window asks.
    /// Answers `{gone, switchedTo}` (aliases or `null`).
    CheckDefaultModel {},
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
    /// Decide a live review; available only to the window and Debug API.
    DecideProtectedChange {
        /// Owning live session.
        session_id: String,
        /// Captured proposal identity.
        proposal_id: String,
        /// Apply all captured files, or decline without writing.
        approve: bool,
        /// Optional plain-text decline feedback.
        #[serde(default)]
        note: Option<String>,
    },
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
    /// Tells the core which session this client's main view shows (`None`: none). A detached session
    /// is only read; a live session keeps its channel open however long it is idle
    /// (`CoreConfig::channel_idle_close`). Viewing never resumes unfinished work.
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
    /// Vibe skills in the selected work folder: list, enable, install, update, remove and detail.
    Skills {
        /// The skills operation.
        request: SkillsRequest,
    },
    /// The reviewed Tools marketplace.
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
    /// Read or watch the saved project's repository tree.
    GitBrowser {
        /// Project browser request.
        request: GitBrowserRequest,
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
    /// App-owned GitHub reader opt-in and account status.
    GithubReads {
        /// Read or set GitHub reads.
        request: crate::GithubReadsRequest,
    },
    /// The "Allow for this session" answers Vibe Work keeps for Vibe's approvals: list, remove one, or remove many.
    AskGrants {
        /// List or remove.
        request: crate::AskGrantsRequest,
    },
    /// Lasting rules old "Always allow" answers left in Vibe's config: list, or remove one (through Vibe's API).
    VibeRules {
        /// List or remove.
        request: crate::VibeRulesRequest,
    },
    /// Chrome Control: the user's permissions, answers on its cards, set up and remove.
    ChromeControl {
        /// What to do.
        request: crate::ChromeControlRequest,
    },
    /// Automatic idle-sleep preference and status.
    KeepAwake {
        /// Read or save the preference.
        request: crate::KeepAwakeRequest,
    },
    /// The New chat screen's terminals and browser before the first message: its folder, the hand-over to the
    /// session the first message started, Close all.
    NewChat {
        /// The operation.
        request: crate::NewChatRequest,
    },
    /// App-owned session log scan settings and actions, without secret values.
    SecretScan {
        /// Scanner operation.
        request: crate::SecretScanRequest,
    },
    /// Safety reminders: the rules the agent's commands are checked against, switched on
    /// or off, and the user's own rules.
    Safety {
        /// What to do.
        request: SafetyRequest,
    },
    /// Settings > Network: the proxy and certificate settings Vibe keeps (`config/proxy/read|write`), a test of the
    /// way to Mistral with them, and saving a change (always with the native confirmation).
    Network {
        /// What to do.
        request: NetworkRequest,
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
