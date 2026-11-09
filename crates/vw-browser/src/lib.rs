//! The agent-controlled browser: observe →
//! act → receipt on a session's built-in browser (the preview engine), the prompt guard on everything a page
//! says, redaction of secret and user-typed fields, and the per-session Stop and action log the user sees.
//!
//! `login_fill` puts a vault login into a sign-in field after ShellX-style document, origin and form checks; the core holds the
//! vault and the grant.
//!
//! User secret choices stay in a one-use native buffer until the window confirms a vault save; page reads hide
//! key-like text (`js/common.js` `hideKeys`).
//!
//! Depends on: vw-preview (the engine and service), serde, tokio, zeroize. Called by: vw-core-api (the `browser_read` /
//! `browser_act` agent tools and the `browser` command for the UI and the Debug API), tests.
//! Invariants:
//! - Page-derived data is untrusted: parsed as data, capped, and returned under `untrustedPageContent`; nothing in
//!   this crate ever follows page text.
//! - A ref action targets an observed node in the same document; secrets are never typed by the agent (a vault login is
//!   filled by the core, never through `type`).
//! - Every operation is bounded in time (see `driver`).

#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!("the consent fixture alias is available only in private debug e2e builds");

mod act;
mod act_notes;
mod action_loop;
pub mod annotate;
mod answers;
pub mod capture;
pub mod choice_context;
mod choices;
pub mod consent;
mod consent_driver;
pub mod delegation;
mod delegation_driver;
pub mod dialogs;
pub mod downloads;
mod driver;
mod frame_wait;
pub mod frames;
pub mod guard;
mod host_action;
mod image_read;
mod inline_output;
#[cfg(test)]
mod inline_output_tests;
mod input_marker;
mod keys;
mod labels;
mod login_fill;
pub mod model;
mod native_focus;
mod navigation_receipt;
pub mod observe;
pub mod output;
mod page;
mod point;
mod point_receipt;
mod pointer;
mod postconditions;
mod read;
pub mod receipt;
mod refs;
pub mod screenshot_area;
pub mod scripts;
mod scroll_shown;
mod secret_capture;
mod sending;
mod sessions;
pub mod site_storage;
pub mod text;
mod text_paging;
mod upload;
mod visual;

pub use act::{
    Action, Covering, DEFAULT_WAIT_MS, KEYS, MAX_TEXT, MAX_UPLOAD_BYTES, MAX_UPLOAD_FILES,
    MAX_WAIT_MS, PageAnswer, Target, UploadFile, parse,
};
pub use action_loop::POST_INPUT_FAILED;
pub use choices::ChoiceCursor;
pub use driver::{ActError, Browser, HEADLINE_KEY, NOTHING_OPEN, NavFuture, Navigate, STOPPED};
pub use login_fill::{FillRefusal, FillTarget, LoginRole};
pub use output::filter_agent_output;
pub use read::ReadOptions;
pub use secret_capture::CapturedSecret;
pub use sending::SEND_REQUIRED;
pub use sessions::{
    ACTIVE_FOR_MS, Actor, BrowserState, LOG_CAP, LogEntry, Outcome, Sessions, now_ms,
};

#[cfg(feature = "agent-vault")]
mod agent_secret;
#[cfg(feature = "agent-vault")]
pub use agent_secret::AgentSecret;
pub use text_paging::TextCursor;
