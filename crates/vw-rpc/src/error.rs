//! Typed errors and the "server is dead" report.
//!
//! Depends on: vw-protocol (`ProtocolError`, the server's error object). Called by: every public
//! API of this crate; the supervisor matches on [`RpcError::ServerDead`] to decide a restart.
//! Invariant: once a connection is dead, every waiting and every later request fails with the
//! same [`DeathReport`] (first cause wins).

use std::fmt;
use std::time::Duration;

use vw_protocol::ProtocolError;

/// How the server process ended, when the caller supplied an exit watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExitInfo {
    /// Exit code, if the process exited normally.
    pub code: Option<i32>,
    /// Terminating signal on Unix (`None` elsewhere or when it exited normally).
    pub signal: Option<i32>,
}

impl fmt::Display for ExitInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.code, self.signal) {
            (Some(code), _) => write!(f, "exit code {code}"),
            (None, Some(signal)) => write!(f, "killed by signal {signal}"),
            (None, None) => f.write_str("exit status unknown"),
        }
    }
}

/// Why the connection to the server is considered dead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeathCause {
    /// stdout reached end of stream (the server exited or closed it).
    Eof,
    /// Reading stdout failed.
    ReadFailed(String),
    /// Writing to stdin failed (broken pipe, closed).
    WriteFailed(String),
    /// A write to stdin did not finish in time: the server stopped reading its input.
    WriteTimedOut {
        /// The write timeout that expired.
        after: Duration,
    },
    /// A request whose silence means the server is wedged got no answer in time (for example,
    /// `callback/result` unanswered, after which Vibe 2.26.0 also exits with code 1).
    Wedged {
        /// Method that went unanswered.
        method: String,
        /// Its request id.
        id: i64,
        /// The timeout that expired.
        after: Duration,
    },
    /// The client closed the connection on purpose (shutdown, or a channel closed by us and
    /// acknowledged by the server).
    ClosedByClient,
    /// The server closed one channel of a multiplexed process on its own (`session/stop`, a full
    /// channel inbox, or the channel's server failed). `error` is the server's text when it sent one
    /// (server output: never logged by this crate).
    ChannelClosed {
        /// The server's `"error"` field of the close frame.
        error: Option<String>,
    },
}

impl DeathCause {
    /// Short stable name of the cause, safe for logs (no I/O error text, no server output).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Eof => "eof",
            Self::ReadFailed(_) => "read_failed",
            Self::WriteFailed(_) => "write_failed",
            Self::WriteTimedOut { .. } => "write_timed_out",
            Self::Wedged { .. } => "wedged",
            Self::ClosedByClient => "closed_by_client",
            Self::ChannelClosed { .. } => "channel_closed",
        }
    }
}

impl fmt::Display for DeathCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("server closed its output"),
            Self::ReadFailed(why) => write!(f, "reading server output failed: {why}"),
            Self::WriteFailed(why) => write!(f, "writing to server failed: {why}"),
            Self::WriteTimedOut { after } => {
                write!(f, "server stopped reading input (write blocked {after:?})")
            }
            Self::Wedged { method, id, after } => {
                write!(
                    f,
                    "server wedged: no answer to {method} (id {id}) within {after:?}"
                )
            }
            Self::ClosedByClient => f.write_str("connection closed by the client"),
            // The server's text stays out of Display (it may be logged); read the field explicitly.
            Self::ChannelClosed { error: None } => f.write_str("the server closed the channel"),
            Self::ChannelClosed { error: Some(_) } => {
                f.write_str("the server closed the channel with an error")
            }
        }
    }
}

/// Everything known about a dead server at the moment it was declared dead.
///
/// `stderr_tail` is sensitive (server output); it is excluded from `Display` and from every log
/// line this crate writes. Configured secrets are already replaced in it (see
/// `ConnectionConfig::stderr_redactions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathReport {
    /// Why it is dead.
    pub cause: DeathCause,
    /// Process exit, when known within the exit grace period (EOF case) or `None`.
    pub exit: Option<ExitInfo>,
    /// The last stderr lines (oldest first), for diagnostics.
    pub stderr_tail: Vec<String>,
}

impl fmt::Display for DeathReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.cause)?;
        if let Some(exit) = self.exit {
            write!(f, " ({exit})")?;
        }
        // The stderr tail is deliberately not part of Display: errors get logged, and server
        // output may hold prompts, file contents or credentials. Read `stderr_tail` explicitly.
        if !self.stderr_tail.is_empty() {
            write!(f, "; {} stderr lines kept", self.stderr_tail.len())?;
        }
        Ok(())
    }
}

/// Error from a request or notification.
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    /// No response within the request's timeout (the server is still considered alive).
    #[error("{method} (id {id}) timed out after {after:?}")]
    Timeout {
        /// Method of the request.
        method: String,
        /// Request id.
        id: i64,
        /// Timeout that expired.
        after: Duration,
    },
    /// The server is dead; see the report for cause, exit status and stderr tail.
    #[error("server dead: {0}")]
    ServerDead(Box<DeathReport>),
    /// The server answered with a JSON-RPC error object.
    #[error("{method} failed: {} ({})", .error.message, .error.code.as_str())]
    Remote {
        /// Method of the request.
        method: String,
        /// The server's error object (`protocol.py:2381 ProtocolError`).
        error: ProtocolError,
    },
    /// Params could not be serialized.
    #[error("encoding params for {method} failed: {source}")]
    Encode {
        /// Method of the request.
        method: String,
        /// Serializer error.
        source: serde_json::Error,
    },
    /// A typed result could not be decoded from the server's answer.
    #[error("decoding the result of {method} failed: {source}")]
    Decode {
        /// Method of the request.
        method: String,
        /// Deserializer error.
        source: serde_json::Error,
    },
}

impl RpcError {
    /// The death report when this error means the server is dead.
    #[must_use]
    pub fn death(&self) -> Option<&DeathReport> {
        match self {
            Self::ServerDead(report) => Some(report),
            _ => None,
        }
    }
}
