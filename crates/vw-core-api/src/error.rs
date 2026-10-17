//! `CoreError`: why a command failed, as one serializable shape for both clients.
//!
//! Depends on: vw-gate (`GateError` and its reasons), vw-rpc (`RpcError`), vw-session
//! (`LoadError`). Called by: `dispatch` (every failure path), vw-debug-api (status codes and error
//! bodies), the UI (generated TypeScript).
//! Invariants:
//! - `code` is stable and machine-readable; `message` is for people.
//! - No params values: gate refusal texts are value-free by the gate's own rule (`vw-gate`
//!   `error.rs`), and this module adds only method names and fixed text. A server error keeps the
//!   server's message, which Vibe writes about its own state.
//! - `GateError::Transport` is the only gate error after which something may have been sent;
//!   it maps to `serverUnavailable`, `timeout` or `serverError`, or keeps the core's own error when the
//!   core itself failed around the request (`RpcError::Local`).

use serde::{Deserialize, Serialize};
use vw_gate::{CallbackFault, GateError};
use vw_rpc::RpcError;

/// Why a core could not start.
#[derive(Debug, thiserror::Error)]
pub enum CoreStartError {
    /// User plugin approvals could not be prepared.
    #[error("{0}")]
    PluginApprovals(String),
    /// An interrupted marketplace update could not be recovered.
    #[error(transparent)]
    Marketplace(#[from] vw_market::MarketError),
    /// Session ownership could not be initialized.
    #[error("{0}")]
    SessionOwnership(String),
    /// The native clipboard worker could not start.
    #[error(transparent)]
    Clipboard(#[from] vw_platform::clipboard::ClipboardError),
    /// The first server start failed (setup problem: path, home, handshake).
    #[error(transparent)]
    Supervisor(#[from] vw_supervisor::SupervisorError),
    /// The process monitor's ledger folder could not be prepared.
    #[error(transparent)]
    Procmon(#[from] vw_procmon::LedgerError),
}

/// Machine-readable error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The gate refused the method or this use of it (default deny, policy).
    Denied,
    /// Params outside the method's typed shape.
    InvalidParams,
    /// The call needs a native confirmation first (`requestConfirmation`).
    ConfirmationRequired,
    /// A confirmation token was given but not accepted.
    ConfirmationRejected,
    /// The user declined the native dialog.
    ConfirmationDeclined,
    /// A confirmation was asked for a call that needs none.
    ConfirmationNotApplicable,
    /// A callback answer was refused (unknown, already answered, wrong kind or shape).
    CallbackRefused,
    /// No session with this id is loaded in the store.
    NotFound,
    /// The command itself is malformed (bad JSON, unknown field, missing field).
    InvalidCommand,
    /// No server is ready (starting, restarting, dead or stopped).
    ServerUnavailable,
    /// The server did not answer in time.
    Timeout,
    /// The server answered with an error.
    ServerError,
    /// Something in the core failed (no randomness, unreadable result).
    Internal,
    /// The core could not put a reopened session back in its access mode, so it closed the session's
    /// channel and sent nothing (fail closed). The next command for it tries again.
    AccessModeUnverified,
    /// The session could not be reopened after a server restart; its commands are refused until the
    /// user tries again (`retryReopen`) or continues in a new session. Nothing was sent.
    SessionStuck,
    /// The user cancelled a long wait (a `retryReopen`).
    Cancelled,
    /// A key or vault operation failed; `server_code` holds a stable reason word
    /// (`wrongPassphrase`, `locked`, `twoVaults`, `unauthorized`, ...). Nothing was sent to Vibe.
    KeyService,
    /// Changes tab: the file or diff is not what the user saw; nothing was changed ("File
    /// changed, refresh").
    Stale,
    /// Changes tab: a revert would delete files the request did not name; `summary` lists them,
    /// one per line. Nothing was changed.
    ConfirmDeletes,
    /// The Vibe install refused a request; `server_code` holds the reason word (`wrongState`,
    /// `staleOffer`, `unsupported`, `unknownExisting`, `needsAcceptance`, `disk`, `notConfigured`).
    VibeInstall,
}

impl ErrorCode {
    /// The wire name (`denied`, `confirmationRequired`, ...).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::InvalidParams => "invalidParams",
            Self::ConfirmationRequired => "confirmationRequired",
            Self::ConfirmationRejected => "confirmationRejected",
            Self::ConfirmationDeclined => "confirmationDeclined",
            Self::ConfirmationNotApplicable => "confirmationNotApplicable",
            Self::CallbackRefused => "callbackRefused",
            Self::NotFound => "notFound",
            Self::InvalidCommand => "invalidCommand",
            Self::ServerUnavailable => "serverUnavailable",
            Self::Timeout => "timeout",
            Self::ServerError => "serverError",
            Self::Internal => "internal",
            Self::AccessModeUnverified => "accessModeUnverified",
            Self::SessionStuck => "sessionStuck",
            Self::Cancelled => "cancelled",
            Self::KeyService => "keyService",
            Self::Stale => "stale",
            Self::ConfirmDeletes => "confirmDeletes",
            Self::VibeInstall => "vibeInstall",
        }
    }

    /// True for refusals by the gate or the core (nothing was sent to Vibe). Only these are published as a
    /// `refused` event (the window shows them as a toast).
    #[must_use]
    pub fn is_refusal(self) -> bool {
        !matches!(
            self,
            Self::ServerUnavailable
                | Self::Timeout
                | Self::ServerError
                | Self::Internal
                // Raised together with its own alert; not reported a second time as a refusal.
                | Self::AccessModeUnverified
                | Self::Cancelled
                // an answer about something that is not there (reading a session that was just
                // deleted, a project id that went away) is an ordinary reply to the caller, not a refusal of
                // its origin, capability or consent; the caller has it in its answer.
                | Self::NotFound
                // A Vibe-install answer (no installer in this run, network, checksum, an offer that is
                // no longer current) goes to the caller and is shown inline by the install step; a failed install
                // is also in the view (`phase: failed`). A refusal toast on top covered the composer in
                // development builds, whose UI reads the status once and gets `notConfigured`.
                | Self::VibeInstall
        )
    }
}

/// `serverCode` of [`CoreError::not_in_this_build`].
pub const NOT_IN_THIS_BUILD: &str = "notInThisBuild";

/// A failed command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct CoreError {
    /// What kind of failure.
    pub code: ErrorCode,
    /// Human-readable description.
    pub message: String,
    /// The JSON-RPC method involved, when there is one.
    pub method: Option<String>,
    /// For `confirmationRequired`: what the native dialog would ask.
    pub summary: Option<String>,
    /// For `serverError`: Vibe's own error code (`not_found`, `stale_turn`, ...); for `keyService`:
    /// the key service's reason word.
    pub server_code: Option<String>,
}

impl CoreError {
    /// An error with only a code and a message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            method: None,
            summary: None,
            server_code: None,
        }
    }

    /// Attaches the method name.
    #[must_use]
    pub fn with_method(mut self, method: &str) -> Self {
        self.method = Some(clip(method, METHOD_MAX));
        self
    }

    /// A feature this build of the core was started without (Project Pages, the preview or the page
    /// viewer in a development host). Code `denied` with `serverCode` [`NOT_IN_THIS_BUILD`]: an answer to the
    /// caller, never a `refused` event (a browser development window asking again on every view showed it as a
    /// stream of "request refused" toasts).
    #[must_use]
    pub fn not_in_this_build(message: impl Into<String>) -> Self {
        let mut error = Self::new(ErrorCode::Denied, message);
        error.server_code = Some(NOT_IN_THIS_BUILD.to_owned());
        error
    }

    /// Whether this failure is published as a `refused` event: a refusal ([`ErrorCode::is_refusal`]) that is not
    /// a missing feature ([`CoreError::not_in_this_build`]).
    #[must_use]
    pub fn is_refusal_event(&self) -> bool {
        self.code.is_refusal() && self.server_code.as_deref() != Some(NOT_IN_THIS_BUILD)
    }

    /// No session `id` is loaded.
    #[must_use]
    pub fn not_loaded(id: &str) -> Self {
        // The id is the caller's own text; bound it so a hostile caller cannot inflate errors.
        let shown: String = id.chars().take(128).collect();
        Self::new(
            ErrorCode::NotFound,
            format!("session {shown:?} is not loaded"),
        )
    }

    /// A server result that does not have the shape the core needs.
    #[must_use]
    pub fn bad_result(method: &str, what: &str) -> Self {
        Self::new(
            ErrorCode::Internal,
            format!("{method}: result has no {what}"),
        )
        .with_method(method)
    }

    /// Maps a transport failure. A failure of the core itself around the request
    /// ([`RpcError::Local`], for example session ownership that cannot be recorded) keeps the core's own
    /// error: its code, message and reason word. It is never reported as a server error.
    #[must_use]
    pub fn from_rpc(method: &str, error: &RpcError) -> Self {
        if let RpcError::Local { source, .. } = error {
            let mut local = source
                .downcast_ref::<Self>()
                .cloned()
                .unwrap_or_else(|| Self::new(ErrorCode::Internal, source.to_string()));
            local.message = clip(&local.message, MESSAGE_MAX);
            if local.method.is_none() {
                local.method = Some(clip(method, METHOD_MAX));
            }
            return local;
        }
        let (code, server_code) = match error {
            RpcError::ServerDead(_) | RpcError::Timeout { id: 0, .. } => {
                (ErrorCode::ServerUnavailable, None)
            }
            RpcError::Timeout { .. } => (ErrorCode::Timeout, None),
            RpcError::Remote { error, .. } => {
                if error
                    .data
                    .as_option()
                    .and_then(|data| data.get("harnessCode"))
                    .and_then(serde_json::Value::as_str)
                    == Some("session_busy")
                {
                    (ErrorCode::Denied, Some("sessionHeld".to_owned()))
                } else {
                    (ErrorCode::ServerError, Some(error.code.as_str().to_owned()))
                }
            }
            RpcError::Encode { .. } | RpcError::Decode { .. } | RpcError::Local { .. } => {
                (ErrorCode::Internal, None)
            }
        };
        let message = match error {
            RpcError::Timeout { id: 0, after, .. } => {
                format!("{method}: no ready server within {after:?}")
            }
            other => other.to_string(),
        };
        Self {
            code,
            message: clip(&message, MESSAGE_MAX),
            method: Some(clip(method, METHOD_MAX)),
            summary: None,
            server_code,
        }
    }

    /// Maps a gate error for `method`.
    #[must_use]
    pub fn from_gate(method: &str, error: &GateError) -> Self {
        let code = match error {
            GateError::Denied { .. } => ErrorCode::Denied,
            GateError::InvalidParams { .. } | GateError::InvalidFolder { .. } => {
                ErrorCode::InvalidParams
            }
            GateError::ConfirmationRequired { .. } => ErrorCode::ConfirmationRequired,
            GateError::ConfirmationRejected { .. } => ErrorCode::ConfirmationRejected,
            GateError::ConfirmationDeclined { .. } => ErrorCode::ConfirmationDeclined,
            GateError::ConfirmationNotApplicable { .. } => ErrorCode::ConfirmationNotApplicable,
            GateError::Callback(fault) => callback_code(fault),
            GateError::Entropy(_) => ErrorCode::Internal,
            GateError::Transport(rpc) => return Self::from_rpc(method, rpc),
        };
        let summary = match error {
            GateError::ConfirmationRequired { summary, .. } => Some(summary.clone()),
            _ => None,
        };
        Self {
            code,
            message: clip(&error.to_string(), MESSAGE_MAX),
            method: Some(clip(method, METHOD_MAX)),
            summary,
            server_code: None,
        }
    }
}

/// Longest method name kept in an error (the caller chose it; a hostile one can be huge).
const METHOD_MAX: usize = 200;
/// Longest message kept in an error.
const MESSAGE_MAX: usize = 1000;

/// The first `max` characters of `text`, with a marker when something was cut.
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("...[cut]");
    out
}

fn callback_code(fault: &CallbackFault) -> ErrorCode {
    match fault {
        CallbackFault::UnknownCallback { .. } => ErrorCode::NotFound,
        _ => ErrorCode::CallbackRefused,
    }
}

#[cfg(test)]
mod tests {
    use super::{CoreError, ErrorCode};
    use vw_rpc::RpcError;

    fn local(error: impl std::error::Error + Send + Sync + 'static) -> RpcError {
        RpcError::Local {
            method: "session/start".to_owned(),
            source: Box::new(error),
        }
    }

    /// The core's own failure keeps its code and plain message; it is never a server error.
    #[test]
    fn a_local_failure_keeps_the_core_error() {
        let own = CoreError::new(
            ErrorCode::Internal,
            "Could not check session ownership: no folder",
        );
        let mapped = CoreError::from_rpc("session/start", &local(own));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.server_code, None);
        assert_eq!(
            mapped.message,
            "Could not check session ownership: no folder"
        );
        assert_eq!(mapped.method.as_deref(), Some("session/start"));
    }

    /// A session another window holds stays a `denied` / `sessionHeld` refusal.
    #[test]
    fn a_held_session_stays_a_held_refusal() {
        let held = crate::session_holder::held_refusal(4242);
        let mapped = CoreError::from_rpc("session/resume", &local(held.clone()));
        assert_eq!(mapped.code, ErrorCode::Denied);
        assert_eq!(mapped.server_code.as_deref(), Some("sessionHeld"));
        assert_eq!(mapped.message, held.message);
        assert_eq!(mapped.method.as_deref(), Some("session/resume"));
    }

    /// Any other local error is the core's internal failure, with its text.
    #[test]
    fn another_local_error_is_internal() {
        let io = std::io::Error::other("disk full");
        let mapped = CoreError::from_rpc("session/fork", &local(io));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.server_code, None);
        assert_eq!(mapped.message, "disk full");
        assert_eq!(mapped.method.as_deref(), Some("session/fork"));
    }
}
