//! The project handbook (Project Handbook Format 1.2, `docs/public/HANDBOOK_FORMAT.md`): the agent
//! keeps the project's plans, decisions, requests, research, reports, roadmap, changelog and guides as linked HTML
//! pages (and Markdown "lite" pages) in `docs/handbook/`; Vibe Work shows them in the Project tab and the Projects
//! overview, validates them (§10), keeps the user's comments on their sections in app data, and shows the plan as a
//! view of the plan page. The same viewer and page server also show one HTML page or PDF from a session's folder for
//! the File tab (`file_view`), in a second service instance with its own webview label.
//!
//! Everything here runs without a window: the viewer webview is driven through vw-preview's `PreviewEngine` seam (a
//! second Tauri engine instance in the app; a recording fake in tests).
//!
//! Depends on: vw-preview, vw-platform, axum (page server), pulldown-cmark (Markdown pages), tokio, serde. Called by:
//! vw-core-api (the `pages` command, the overview, the agent's `handbook_check`, the `vw-plan` profile, preview
//! element picks), the app (engine setup), tests.
//! Invariants:
//! - Pages and `index.html` are the agent's. Only the explicit `handbook_check({repair: "stylesheet"})` tool writes
//!   the bundled `handbook.css` into a repository; viewers/validators never write. Pre-1.0 `vw-*` names are read only.
//! - Agent pages are untrusted: served with a CSP that blocks network beyond the page server, shown only in the
//!   isolated engine, navigation limited to the page server; their text is length-capped data.
//! - Section identity comes from the page FILE, never from what a page script reports.

mod answering;
mod answers;
mod checker;
mod checking;
mod comments;
mod config;
mod csp;
mod export;
mod export_clean;
mod export_files;
mod file_view;
pub mod handbook;
mod html;
mod hub;
mod inline;
mod links;
mod lite;
mod markdown;
mod opening;
mod page;
mod page_inputs;
mod page_scan;
mod paths;
mod pick;
mod picking;
mod profile;
mod projects;
mod server;
mod service;
mod settings;
mod template;
mod theme;
mod viewer;

pub use answers::{
    Answer, AnswerError, AnswerStore, AnswerValue, InputAnswer, PageAnswers, fit as answer_fits,
    page_answers,
};
pub use checker::{CHECK_LABEL_PREFIX, ISSUE_KINDS, VisualChecker, VisualIssue, VisualReport};
pub use comments::{Comment, CommentError, CommentStore, MAX_QUOTE, MAX_TEXT, project_key};
pub use config::{FILES_LABEL_PREFIX, PagesConfig, PagesError, PickedSection};
pub use export::{Exported, MAX_ASSET, MAX_EMBEDDED, MAX_OUTPUT, export as export_page};
pub use handbook::files::inside;
pub use handbook::overview::{HandbookState, Manifest, PlanSummary, ProjectCard};
pub use handbook::validate::{Finding, Severity, Validation, validate};
pub use html::{Tag, Token, decode_entities, escape, tokens};
pub use hub::{GROUPS, Hub, HubGroup, HubPage, IndexHealth, build as build_hub, live_index_html};
pub use links::{Target, has_scheme, resolve};
pub use lite::{
    INFERRED_FOLDERS, parse_markdown, parse_markdown_at, scan_markdown, scan_markdown_at,
};
pub use markdown::{
    FrontMatter, outline, render as render_markdown, render_at as render_markdown_at, slug,
    split_front_matter,
};
pub use page::{
    Input, InputOption, LegacyHead, PageDetail, PageFacts, PageScan, Reference, Relation,
    STEP_STATES, Section, Step, normalize_step, parse_html, scan_html,
};
pub use paths::{
    DEFAULT_FOLDER, INDEX, PagePath, PathError, asset_allowed, clean_relative, is_page_name,
};
pub use pick::{
    PickAnswer, PickAt, Picked, TargetMiss, accessor_name as pick_accessor, cancel_script,
    parse as parse_pick, parse_target, poll_script, start_script, target_script,
};
pub use picking::{run_pick, section_of};
pub use profile::{
    NAME as PLAN_PROFILE, PlanTarget, pattern as plan_pattern, toml as plan_profile_toml,
};
pub use server::{CHECK_PREFIX, ERROR_PRELUDE, MAX_FILE, PagesServer, Policy, csp};
pub use service::{LABEL_PREFIX, PagesService};
pub use settings::{PagesSettings, SettingsStore};
pub use template::{STYLE, document};
pub use theme::theme_script;
pub use viewer::{ViewerView, allowed as viewer_allows};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "project handbook: format, validator, hub, overview, page server, comments, plan profile, viewer";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
