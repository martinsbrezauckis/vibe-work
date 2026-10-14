//! JSON-RPC 2.0 over stdio for `vibe-app-server`: NDJSON framing, request/response matching,
//! server-to-client requests and notification fan-out.
//!
//! Depends on: vw-protocol (error object, method registry), tokio, `serde_json`.
//! Called by: vw-supervisor (one [`Connection`] per server process); later vw-session/vw-gate.
//! Invariants:
//! - Every request either gets its answer or a typed error (timeout, remote error, server dead);
//!   nothing is fire-and-forget and nothing waits forever.
//! - Request ids are process-wide and never reused; channel ids are per server process, from 1,
//!   never reused within it.
//! - A slow subscriber or handler never blocks the reader; a bad frame never ends the connection.
//! - stderr is diagnostics in a bounded ring, never protocol.
//!
//! The crate knows nothing about processes: the caller hands it the server's stdout, stdin and
//! (optionally) stderr plus an exit-status watch. That keeps it testable over in-memory pipes.
//! [`Multiplexer`] runs the same per-connection logic for every channel of one
//! `vibe-app-server --multiplex` process: each channel is a [`Connection`].
//!
//! ```no_run
//! # async fn demo(stdout: tokio::io::DuplexStream, stdin: tokio::io::DuplexStream) {
//! use vw_rpc::{ConnectionBuilder, ConnectionConfig};
//! let conn = ConnectionBuilder::new(ConnectionConfig::default()).start(stdout, stdin);
//! let result = conn.request("session/list", serde_json::json!({"limit": 5})).await;
//! # let _ = result;
//! # }
//! ```

mod config;
mod connection;
mod error;
mod framing;
mod handler;
mod message;
mod mux;
mod reader;
mod shared;
mod stderr_ring;
mod writer;

pub use config::{
    ConnectionConfig, DEFAULT_LONG_METHODS, DEFAULT_WEDGE_METHODS, RequestOptions, TimeoutPolicy,
};
pub use connection::{Connection, ConnectionBuilder, Subscription, TransportStats, WeakConnection};
pub use error::{DeathCause, DeathReport, ExitInfo, RpcError};
pub use framing::{BoundedLine, encode_frame, read_bounded_line};
pub use handler::{HandlerError, HandlerFuture, ServerRequestHandler};
pub use message::{GapReason, Inbound, Notification, ServerRequest};
pub use mux::{MAX_CHANNEL_ID, Multiplexer, MultiplexerBuilder, MuxStats};
pub use stderr_ring::{REDACTED, Redactions, StderrLimits, StderrRing, TRUNCATION_MARKER};

/// One-line statement of what this crate is responsible for, exposed so tests and diagnostics
/// can assert which crate they are looking at.
pub const CRATE_ROLE: &str = "JSON-RPC framing and request matching";

/// Returns the crate version from Cargo metadata (the workspace version).
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
