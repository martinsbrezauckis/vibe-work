//! The Debug API: a loopback HTTP + SSE server through which an external
//! agent or a test drives Vibe Work exactly like the UI does, as a second client of the core's
//! command API and therefore through the same method gate.
//!
//! Crate placement: its own crate on top of vw-core-api, so the HTTP stack (axum, hyper) is not
//! a dependency of the core or of the Tauri bridge. ShellX patterns adapted from
//! `src-tauri/src/debug_api*.rs` and `loopback_security.rs` @ e38c2b5c.
//!
//! Depends on: vw-core-api, vw-platform, axum 0.8, tokio. Called by: the app (wires it from
//! Settings), tests, the private debug host.
//! Invariants:
//! - Bound to `127.0.0.1` on a random port; off means not bound at all ([`DebugApiConfig`]).
//! - Every route is guarded in this order: Host, Origin (and browser fetch metadata), bearer token
//!   (constant-time). There are no exempt routes and no query-string tokens.
//! - The token is one stable, user-only file per user (`debug-api.token`, reused across runs,
//!   rotatable); the per-run descriptor holds the port and the token file's path, never the token.
//! - No route returns a secret.

// the end-to-end runs' in-page channel exists only in debug builds made for them. A release build that
// asks for it does not compile, so no shipped binary can carry it (the release check proves it).
#[cfg(all(feature = "e2e-script", not(debug_assertions)))]
compile_error!(
    "the e2e-script feature is for debug test builds only; it must never be in a release build"
);

// the vault test switch, the same rule (private e2e debug builds only).
#[cfg(all(feature = "vault-e2e", not(debug_assertions)))]
compile_error!(
    "the vault-e2e feature is for debug test builds only; it must never be in a release build"
);

mod ask_routes;
mod auth;
mod config;
/// Test builds only (feature `e2e-script`): `POST /e2e/script`, `GET /e2e/windows`.
#[cfg(feature = "e2e-script")]
pub mod e2e_script;
mod registry;
mod reply;
mod routes;
mod secret_guard;
mod server;
mod sse;
mod terminal_routes;
mod token;
mod trace_routes;
/// Private test builds only (feature `vault-e2e`): the vault test switch's routes; the source is outside
/// the exported tree.
#[cfg(feature = "vault-e2e")]
// rustfmt must not follow it: the file is absent from the public export, where `cargo fmt` would fail on it.
#[rustfmt::skip]
#[path = "../../../private/vault-e2e/routes.rs"]
mod vault_e2e;
mod window_routes;

pub use auth::{constant_time_eq, host_allowed, origin_allowed, token_present};
pub use config::{DEV_BUILD, DebugApiConfig, EnabledReason, enablement};
pub use registry::{RegistryError, UiRoutes};
pub use reply::status_of;
pub use server::{DebugApi, DebugApiError};
pub use token::{TOKEN_FILE, rotate_token_file};
pub use window_routes::{
    AppWindow, BoxFuture, SharedAppWindow, WindowRequest, WindowState, WindowView, listed_dialog,
};

/// Version of the route set (bumped when a route changes incompatibly).
pub const API_VERSION: &str = "1";

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "loopback Debug API over the core command API";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
