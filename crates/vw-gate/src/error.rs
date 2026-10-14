//! Typed errors of the gate: why a request or an answer was refused, or why sending failed.
//!
//! Depends on: vw-rpc (`RpcError`), `thiserror`. Called by: every module of this crate; the app
//! maps these to UI messages and Debug API status codes.
//! Invariant: every variant except [`GateError::Transport`] means *nothing was written to the
//! server*. The refusal text never contains params values (they may hold prompts or paths).

use vw_rpc::RpcError;

/// Why the gate refuses a method outright (class "denied", or no class at all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// The method is not in the table (default deny). A new Vibe method lands here until it is
    /// classified.
    Unclassified,
    /// Used by our own code only (`initialize` belongs to the supervisor handshake).
    InternalOnly,
    /// Must go through a dedicated gate entry point with extra checks (`callback/result` ->
    /// `Gate::answer_callback`).
    DedicatedEntryPoint,
    /// Would put a secret into Vibe's hands or disk; the OS keychain owns it.
    Credential,
    /// Sends usage data off the machine; off unless a later Privacy switch enables it.
    Privacy,
    /// Exists only on the legacy harness; Vibe Work runs the unified one.
    NotOnUnified,
    /// Answers `method_not_found` on both harnesses (bench report).
    NotSupported,
    /// Never exercised in the bench, so its behaviour on the unified harness is unknown.
    NoEvidence,
    /// Removes git worktrees, dirty ones included, without the files they hold being named to the user
    /// (`workspace/git/worktrees/prune` and `reap`). Vibe Work's own cleanup inspects first and
    /// asks with the file list (`worktrees` command).
    UnnamedRemoval,
    /// A request of an allowed method that a policy forbids (path outside the allowlist, option
    /// that runs code, folder the user did not pick). The text names the rule, not the values.
    Policy(String),
}

impl std::fmt::Display for DenyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unclassified => f.write_str("unknown method (default deny)"),
            Self::InternalOnly => f.write_str("reserved for the core"),
            Self::DedicatedEntryPoint => f.write_str("has its own validated entry point"),
            Self::Credential => f.write_str("credentials are owned by the OS keychain"),
            Self::Privacy => f.write_str("sends usage data; disabled by privacy policy"),
            Self::NotOnUnified => f.write_str("not available on the unified harness"),
            Self::NotSupported => f.write_str("not supported by the server"),
            Self::NoEvidence => f.write_str("not verified on the unified harness"),
            Self::UnnamedRemoval => f.write_str(
                "removes worktrees without naming their files; use the app's worktree cleanup",
            ),
            Self::Policy(rule) => write!(f, "refused by policy: {rule}"),
        }
    }
}

/// Why a confirmation token was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenFault {
    /// No such token (never issued, forged, evicted, or already swept).
    Unknown,
    /// Issued for another method or other params.
    Mismatch,
    /// Older than its lifetime.
    Expired,
    /// Already used once.
    Reused,
}

impl std::fmt::Display for TokenFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unknown => "unknown token",
            Self::Mismatch => "token was issued for a different request",
            Self::Expired => "token expired",
            Self::Reused => "token already used",
        })
    }
}

/// Why an answer to a server callback was refused (nothing was sent).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackFault {
    /// No open callback with this id.
    #[error("no open callback with id {id}")]
    UnknownCallback {
        /// The id the caller gave.
        id: String,
    },
    /// The callback was already answered (or is being answered).
    #[error("callback {id} was already answered")]
    AlreadyResolved {
        /// The callback id.
        id: String,
    },
    /// The callback belongs to another session.
    #[error("callback {id} belongs to a different session")]
    WrongSession {
        /// The callback id.
        id: String,
    },
    /// The answer is for a different kind of callback (Vibe accepts this and then
    /// conflicts with the real answer).
    #[error("answer is for a `{got}` callback but this one is `{expected}`")]
    WrongKind {
        /// Kind of the open callback.
        expected: &'static str,
        /// Kind the answer claims.
        got: String,
    },
    /// The answer does not have the exact shape for the callback's kind (such an
    /// answer wedges the server).
    #[error("malformed answer: {detail}")]
    Malformed {
        /// What is wrong, without echoing values.
        detail: String,
    },
    /// The callback kind has no verified answer shape (`connector_auth`, unknown kinds); the UI
    /// is shown the callback but the gate does not answer it.
    #[error("callbacks of kind `{kind}` cannot be answered through the gate")]
    UnsupportedKind {
        /// The callback's kind.
        kind: String,
    },
    /// The `callback/call` params were not a callback entry.
    #[error("not a callback entry: {detail}")]
    InvalidRegistration {
        /// What is wrong.
        detail: String,
    },
    /// Too many callbacks are open at once.
    #[error("more than {limit} callbacks are open")]
    TooManyOpen {
        /// The configured limit.
        limit: usize,
    },
    /// The id belongs to a callback that was already answered.
    #[error("callback id {id} was already used")]
    DuplicateId {
        /// The callback id.
        id: String,
    },
}

/// Why a call through the gate did not happen, or failed.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// The method (or this use of it) is not allowed.
    #[error("{method} is denied: {reason}")]
    Denied {
        /// The requested method.
        method: String,
        /// Why.
        reason: DenyReason,
    },
    /// The params do not match the method's typed shape.
    #[error("{method}: invalid params: {detail}")]
    InvalidParams {
        /// The requested method.
        method: String,
        /// What is wrong, without echoing values.
        detail: String,
    },
    /// The call needs a native confirmation; no token was supplied.
    #[error("{method} needs a native confirmation: {summary}")]
    ConfirmationRequired {
        /// The requested method.
        method: String,
        /// What the user would be asked.
        summary: String,
    },
    /// A token was supplied but not accepted.
    #[error("{method}: confirmation token refused: {fault}")]
    ConfirmationRejected {
        /// The requested method.
        method: String,
        /// Why.
        fault: TokenFault,
    },
    /// The user declined in the native dialog.
    #[error("{method}: declined by the user")]
    ConfirmationDeclined {
        /// The requested method.
        method: String,
    },
    /// A confirmation was requested for a call that needs none.
    #[error("{method}: this call needs no confirmation")]
    ConfirmationNotApplicable {
        /// The requested method.
        method: String,
    },
    /// A callback answer was refused.
    #[error(transparent)]
    Callback(#[from] CallbackFault),
    /// The folder cannot be registered as user-picked.
    #[error("cannot use folder {path}: {reason}")]
    InvalidFolder {
        /// The folder as given.
        path: String,
        /// Why.
        reason: String,
    },
    /// The OS random source failed, so no token or id could be made.
    #[error("no randomness available: {0}")]
    Entropy(String),
    /// The request was sent and failed (server error, timeout, dead server).
    #[error(transparent)]
    Transport(#[from] RpcError),
}
