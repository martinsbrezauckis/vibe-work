//! Session state of record, JSON-Patch apply and event fan-out.
//!
//! The Rust core's state for every open Vibe session, rebuilt only from `session/start|resume|read`
//! results plus server notifications, and fanned out to subscribers (UI, Debug API) as snapshots
//! followed by ordered changes. Pure logic and no I/O: the transport feeds it; when it needs a
//! re-read of a session it says so with [`Action::NeedsResync`] and the caller does `session/read`.
//!
//! Entry points: [`SessionStore::apply`] (notifications), [`SessionStore::load`] (request results),
//! [`SessionStore::view`] / [`SessionStore::subscribe`] (reads), [`SessionStore::reconcile_listing`]
//! (sidebar). State is kept as wire JSON so fields this client does not model survive untouched.
//!
//! Depends on: `serde_json`, `thiserror`. Called by: vw-gate, app (UI bridge, Debug API).
//! Invariant: state is owned by this crate; other crates get read views or send commands.

mod anomaly;
mod change;
mod fanout;
mod lifecycle;
mod patch;
mod reducer;
mod session;
mod store;

pub use anomaly::{Anomalies, AnomalyCounts, AnomalyKind, AnomalyRecord};
pub use change::{Action, Applied, Change, Notification, SessionView};
pub use fanout::{Delivery, Subscription, Waker};
pub use lifecycle::{LoadError, LoadKind, SessionRow};
pub use patch::{PatchError, apply_patch};
pub use store::SessionStore;

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "session state of record and event fan-out";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
