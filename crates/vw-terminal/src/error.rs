//! Typed errors of the terminal manager, so the core can map each one to a stable error code.
//!
//! Depends on: `types` (`TerminalId`). Called by: every module that can fail; the core maps
//! [`TerminalError`] to its `ErrorCode`.
//! Invariant: messages name the terminal, session or folder involved, never the text a user typed.

use crate::types::TerminalId;

/// Why a terminal operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    /// No terminal with that id (never opened, or closed).
    #[error("no terminal {0}")]
    NotFound(TerminalId),
    /// The session already has the most terminals it may have.
    #[error("session {session_id} already has {max} terminals open (close one first)")]
    TooMany {
        /// The session.
        session_id: String,
        /// The cap.
        max: usize,
    },
    /// The session's folder does not exist (moved, deleted, or an unplugged drive).
    #[error("the session folder {0} does not exist")]
    MissingFolder(String),
    /// The terminal's shell has exited; restart it to type again.
    #[error("terminal {0} is not running (restart it)")]
    NotRunning(TerminalId),
    /// Input is queued faster than the program reads it.
    #[error("terminal {0} is not reading its input; try again in a moment")]
    InputBusy(TerminalId),
    /// A size outside 2..=1000 columns or 1..=500 rows.
    #[error("terminal size {cols}x{rows} is out of range")]
    BadSize {
        /// Columns asked for.
        cols: u16,
        /// Rows asked for.
        rows: u16,
    },
    /// The manager is shutting down (app quit) and opens nothing new.
    #[error("terminals are shutting down")]
    ShuttingDown,
    /// The PTY or the shell could not be started.
    #[error("cannot start a shell for session {session_id}: {reason}")]
    Spawn {
        /// The session.
        session_id: String,
        /// The OS or library message.
        reason: String,
    },
    /// The process tree did not end within the close timeout.
    #[error("terminal {id} did not stop within {seconds} s{detail}")]
    StopTimeout {
        /// The terminal.
        id: TerminalId,
        /// The timeout.
        seconds: u64,
        /// Termination errors, when there were any (starts with "; ").
        detail: String,
    },
    /// Any other OS failure (resize, write).
    #[error("terminal {id}: {reason}")]
    Io {
        /// The terminal.
        id: TerminalId,
        /// The OS message.
        reason: String,
    },
}
