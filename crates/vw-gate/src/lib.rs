//! Method gate: the only door from the UI and the Debug API to Vibe.
//!
//! Depends on: vw-rpc (transport), vw-protocol (types, method map), `serde_json`, getrandom. The
//! other `vw-*` crates are linked for the dependency graph.
//! Called by: app (UI commands, Debug API routes, the `callback/call` handler).
//! Invariants:
//! - Default deny: every client request method is in one table with exactly one class; a method
//!   that is not in it is refused and nothing is written to the transport.
//! - Four classes: *forwarded* (typed allowlist), *privileged* (typed handler per method),
//!   *confirmed* (needs a single-use token issued after a native dialog, bound to the exact
//!   method and params), *denied*.
//! - Answers to server callbacks pass the gate too: kind and exact output shape are checked
//!   before anything is sent.
//! - The webview is untrusted: nothing in this crate trusts a value because the UI
//!   sent it.
//!
//! ```no_run
//! # async fn demo(conn: vw_rpc::Connection, confirmer: impl vw_gate::Confirmer) {
//! use vw_gate::{Gate, GateConfig};
//! let gate = Gate::new(conn, confirmer, GateConfig::default());
//! let reply = gate
//!     .call("turn/start", serde_json::json!({"sessionId": "s1", "message": []}), None)
//!     .await;
//! # let _ = reply;
//! # }
//! ```

mod agent_config;
mod callback_shapes;
mod callbacks;
mod canonical;
mod class;
mod config_rules;
mod confirm;
mod confirm_text;
mod content;
mod dialog;
mod error;
mod gate;
mod ids;
mod inbound;
mod mcp_url;
mod picked;
mod privileged;
mod profile;
mod proxy_write;
mod runtime_view;
mod session_rules;
mod shape;
mod table;
mod tool_rules;
mod transport;

pub use callbacks::{CallbackKind, OpenCallback};
pub use class::{Class, P1};
pub use confirm::{
    Clock, Confirmation, ConfirmationRequest, ConfirmationToken, Confirmer, Requester, SystemClock,
};
pub use content::{MAX_IMAGE_BYTES, MAX_IMAGES, MAX_TEXT_BYTES};
pub use dialog::{
    dialog_chat_quote, dialog_command, dialog_label, dialog_line, dialog_quote, dialog_value,
    is_hidden_char, is_quote_char,
};
pub use error::{CallbackFault, DenyReason, GateError, TokenFault};
pub use gate::{Gate, GateConfig, OFFERED_INSTALL_METHODS, Reply, TRUSTED_SURFACE_METHODS};
pub use inbound::{InboundKind, classify_server_request};
pub use mcp_url::{MAX_MCP_URL_LENGTH, McpUrlRefusal, check_mcp_server_url};
pub use profile::full_access_escalation;
pub use table::{classified_methods, classify, forwarded_without_unified_evidence};
pub use tool_rules::ToolRuleRemoval;
pub use transport::Transport;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "method gate: allowlist plus typed privileged handlers";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
