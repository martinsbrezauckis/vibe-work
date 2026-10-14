//! The core's command API and its one dispatcher.
//!
//! The UI (over Tauri IPC) and the Debug API are two clients of one API: [`Command`] in,
//! [`CommandResult`] or [`CoreError`] out, [`CoreEvent`]s on a [`CoreSubscription`]. Every command
//! is executed by [`Core::execute`], and every one that reaches Vibe goes through the method gate
//! TypeScript types for the UI are generated from these Rust types (feature
//! `ts`, binary `gen-core-ts`, output `ui/src/protocol/core/`).
//!
//! The core is a separate crate above vw-gate. The gate stays a pure policy crate over a
//! one-method transport; the core adds process
//! supervision, the store and the event pump, which the gate must not depend on. Dependency
//! direction: `app`/`vw-debug-api` -> `vw-core-api` -> `vw-gate` -> `vw-session` -> `vw-rpc` ->
//! `vw-protocol`, with `vw-supervisor` used by the core.
//!
//! Depends on: vw-gate, vw-session, vw-supervisor, vw-rpc, vw-protocol, vw-procmon (process and port
//! monitor), vw-keys (key and vault commands), vw-changes (Changes tab), vw-documents
//! (documents), vw-mistral (dictation audio), vw-preview (preview pane), vw-pages (Project Pages,
//! the File tab's viewer), vw-worktrees (worktree per session), tokio, serde.
//! Called by: vw-debug-api, app, tests.
//! Invariants:
//! - No path to Vibe that bypasses the gate: the core holds the gate and never the raw
//!   connection for requests (the connection is used only to subscribe to notifications).
//! - Every command carries its [`Origin`], logged with every refusal.

// The vault test switch exists only in debug builds made for the private e2e runs. A release build that
// asks for it does not compile, so no shipped binary can carry it (the private release-binary check proves it).
#[cfg(all(feature = "vault-e2e", not(debug_assertions)))]
compile_error!(
    "the vault-e2e feature is for debug test builds only; it must never be in a release build"
);

#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!("the e2e-script feature is for debug test builds only");

mod access;
mod agent_access;
pub(crate) mod agent_tools;
mod attachment_save;
mod background_output;
mod browser;
mod browser_accounts;
mod browser_capture;
mod browser_consent;
mod browser_delegation;
mod browser_delegation_receipts;
mod browser_fill;
mod browser_guard;
mod browser_secret;
mod browser_tab_tools;
mod browser_tabs;
mod browser_view;
mod changes;
mod changes_facts;
mod changes_ops;
mod changes_slots;
mod changes_view;
mod channels;
mod command;
mod confirmer;
mod connector_consent;
mod connectors;
mod connectors_config;
mod connectors_project;
mod connectors_view;
mod core;
mod default_model;
mod dialog_texts;
mod dictation;
mod dispatch;
mod dispatch_compaction;
mod document_save;
mod documents;
mod documents_view;
mod entry_permissions;
mod entry_projects;
mod error;
mod event;
mod export_md;
mod files;
mod files_read;
pub mod files_scope;
mod files_view;
mod files_write;
mod fill_grants;
mod git_api;
mod git_card;
mod git_cmd;
mod git_discovery;
mod git_draft;
mod git_pr;
mod git_publish;
mod git_settings;
mod git_view;
mod handoff;
mod instructions;
mod instructions_view;
mod key_replace;
mod keys;
mod left_running;
mod left_running_view;
mod line_comments;
mod line_comments_view;
mod market;
mod market_project;
mod market_view;
mod mcp_keys;
mod mcp_keys_config;
mod names;
mod one_ask;
mod pages;
mod pages_answers;
mod pages_data;
mod pages_look;
mod pages_more;
mod pages_overview;
mod pages_plan;
mod pages_share;
mod pages_view;
mod plan;
mod plan_profile;
mod preview_pick;
mod preview_ports;
mod preview_view;
mod previews;
mod processes;
mod project_instruction_files;
mod project_instruction_links;
mod project_instruction_ops;
mod project_instructions;
mod project_instructions_view;
mod projects;
mod projects_filing;
mod projects_view;
mod pump;
mod recovery;
mod restore;
mod resync_budget;
mod routing;
mod safety;
mod safety_view;
mod save_file;
mod search;
mod search_files;
mod search_index;
mod search_text;
mod search_view;
mod secret_generate;
mod secret_grants;
mod secret_replace;
mod secret_scrub;
mod secrets;
mod secrets_view;
mod session_end;
mod session_export;
mod session_folders;
mod session_holder;
mod session_processes;
mod session_tools;
mod session_tools_view;
mod sessions;
mod speech;
mod speech_view;
mod stuck;
mod subagents_setting;
mod subscription;
mod temporary;
mod terminal_procmon;
mod terminal_view;
mod terminals;
mod transport;
mod vault_scope;
mod vibe_install;
mod view;
mod work;
mod work_view;
pub mod working_note;
mod worktrees;
mod worktrees_remove;
mod worktrees_start;
mod worktrees_view;

#[cfg(feature = "ts")]
pub mod ts;

pub use agent_tools::{AgentToolsConfig, BeforeStart};
pub use browser_view::{BrowserRequest, BrowserResult, BrowserShot};
pub use changes_facts::path_key;
pub use changes_slots::ChangesTestHooks;
pub use changes_view::{
    ChangesMode, ChangesRequest, ChangesResult, ChangesView, FolderSession, TurnChanges, TurnFile,
    UndoPreview,
};
pub use command::{Command, CommandResult, Origin};
pub use connectors_view::{
    CatalogLoad, ConnectorView, ConnectorsRequest, ConnectorsResult, ConnectorsView,
    NewServerSecret, ServerKeyView, ServerView, ToolView,
};
pub use core::{Core, CoreConfig, CoreStartError};
pub use dictation::SpeechConfig;
pub use dispatch::invalid_command;
pub use documents::DocumentsConfig;
pub use documents_view::{
    DocumentRefusal, DocumentSource, DocumentView, DocumentsRequest, DocumentsResult, DroppedFile,
    KeptUnitView, KeptView,
};
pub use error::{CoreError, ErrorCode};
pub use event::{AlertKind, CoreEvent, SessionChange, SubagentCard};
pub use files_read::{FileKind, FileRead, IMAGE_LIMIT, TEXT_LIMIT, fingerprint, read_file};
pub use files_view::{FileContent, FileRefView, FilesRequest, FilesResult, MAX_RESOLVE};
pub use git_cmd::DEFAULT_COMMIT_MODEL;
pub use git_view::{
    BringPlanView, CommitPlanView, DraftSource, GitLocation, GitRepoView, GitRequest, GitResult,
    GitSettings, GitStatusView, PrCreatedView, PrPlanView, PrSource, PrStatusView, PrVia,
    PreparedKind, PushPlanView,
};
pub use instructions::InstructionsConfig;
pub use instructions_view::{
    InstructionsDoc, InstructionsRequest, InstructionsResult, InstructionsScope,
};
pub use keys::{
    KeyCandidateView, KeyMetadataView, KeyRequest, KeySourceView, KeyStatusView, RateLimitsView,
    SecretInput, VaultChoice, VibeKeySource, VibeKeyView,
};
pub use left_running_view::{
    LeftRunningView, ProcessesRequest, ProcessesResult, SessionProcessView,
};
pub use line_comments_view::{
    LineCommentState, LineCommentView, LineCommentsRequest, LineCommentsResult,
};
pub use market::MarketConfig;
pub use market_view::{
    CategoryView, ComponentView, DroppedView, InstallProgress, InstalledView, MarketEntryView,
    MarketRequest, MarketResult, MarketView, SessionPluginView, SkillStateView,
};
/// The native dialog's method name for giving an MCP server a vault secret (the host labels its buttons).
pub use mcp_keys::METHOD as MCP_KEY_DIALOG_METHOD;
pub use pages::PagesSetup;
pub use pages_view::{PagesRequest, PagesResult, ProjectHub};
pub use preview_view::{BusyOption, BusyPortView, PreviewRequest, PreviewResult, PreviewTarget};
pub use previews::PreviewSetup;
pub use project_instructions_view::{
    ProjectInstructionFile, ProjectInstructionsRequest, ProjectInstructionsResult,
};
pub use projects_view::{ProjectView, ProjectsRequest, ProjectsResult};
pub use resync_budget::{Budget, ResyncBudget};
pub use safety_view::{CustomRuleView, SafetyRequest, SafetyResult, SafetyRuleView};
pub use search_view::{FileHit, IndexStatus, MessageHit, SearchRequest, SearchResult};
pub use secrets_view::{
    EntryPermissionView, FillGrantView, GrantView, LoginPassword, LoginView, PasswordOptions,
    ProjectSecretView, SecretUseView, SecretsRequest, SecretsResult, ServerGrantView,
};
pub use session_tools_view::{SessionExportView, SessionToolsRequest, SessionToolsResult};
pub use speech::LANGUAGES;
pub use speech_view::{DictationMode, DictationUpdate, SpeechRequest, SpeechResult, VoiceView};
pub use subscription::CoreSubscription;
pub use terminal_view::{
    TerminalEventView, TerminalOutputView, TerminalRequest, TerminalResult, TerminalStateView,
    TerminalView,
};
/// The vault rule for the Debug API (the table, the native-only dialogs, the refusal word).
pub use vault_scope::{
    ACTS as VAULT_ACTS, AGENT_ONLY_DIALOG_METHODS as VAULT_AGENT_ONLY_DIALOG_METHODS,
    DIALOG_METHODS as VAULT_DIALOG_METHODS, VAULT_USER_ONLY, VAULT_USER_ONLY_MESSAGE, VaultAccess,
    vault_access, vault_act,
};
pub use vibe_install::VibeRequest;
pub use view::{
    AnomalyCountsView, AnomalyRecordView, AnomalyReport, CallbackInfo, ChannelView, ChannelsReport,
    CoreStatus, DeathView, DebugApiFlag, RecoveringView, ServerReport, ServerStatus,
    SessionRowView, SessionSnapshot, StuckView,
};
pub use vw_terminal::{AttachInfo, TerminalConfig, ViewerSink};
pub use work::{WorkConfig, image_costs, library_costs, mime_of, tokens_of};
pub use work_view::{
    ImagesRequest, ImagesResult, LibrariesRequest, LibrariesResult, LibraryDocumentView,
    LibraryFile, LibraryScope, LibrarySourceView, LibraryUploadRefusal, LibraryView, WorkCosts,
};
pub use worktrees_view::{
    BranchPlan, OrphanWorktree, RemoveOutcome, WorktreeInfo, WorktreeInspection, WorktreeOptions,
    WorktreeState, WorktreesRequest, WorktreesResult,
};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "core command API and its gated dispatcher";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
