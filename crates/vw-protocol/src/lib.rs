//! Protocol types for the Vibe app-server (JSON-RPC over stdio). All protocol types live in the
//! `generated` module tree, produced by `tools/public/codegen` from Vibe's own Pydantic models;
//! this crate has no hand-written protocol types and none may be added here.
//!
//! Depends on: `serde` and `serde_json` only (it is the bottom of our dependency graph).
//! Called by: vw-rpc, vw-session, vw-gate, app.
//! Invariants: the `generated` tree is never edited by hand and CI fails on any drift from the
//! pinned Vibe version; unknown enum values and union tags decode to `Unknown` variants instead of
//! failing, so a newer Vibe does not break this client.

mod generated;

pub use generated::*;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "generated Vibe app-server protocol types";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
