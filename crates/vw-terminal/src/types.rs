//! The data the manager hands out: ids, open requests, terminal descriptions, output reads, the
//! attach answer and lifecycle events.
//!
//! Depends on: std. Called by: every module of this crate, the core (maps these to its wire views).
//! Invariant: none of these carries input the user typed or anything from the environment.

use std::fmt;
use std::path::PathBuf;

/// A terminal's id (`term-<n>`, unique for the life of the process).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    /// Wraps an id received from a client (unknown ids simply match nothing).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What [`crate::TerminalManager::open`] needs.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    /// The session the terminal belongs to.
    pub session_id: String,
    /// The session's folder (from the core's record of the session, never from a client).
    pub cwd: PathBuf,
    /// Initial columns (the view fits and resizes right after).
    pub cols: u16,
    /// Initial rows.
    pub rows: u16,
}

/// How a shell ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitInfo {
    /// The exit code (Windows codes are 32-bit unsigned). Unix: 1 when killed by a signal.
    pub exit_code: Option<u32>,
    /// The signal name when a Unix shell was killed by one.
    pub signal: Option<String>,
}

/// Whether a terminal's shell runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalState {
    /// The shell runs.
    Running,
    /// The shell ended; the terminal keeps its scrollback until it is restarted or closed.
    Exited(ExitInfo),
}

/// One terminal as clients see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalInfo {
    /// Its id.
    pub id: TerminalId,
    /// Its session.
    pub session_id: String,
    /// The folder it started in.
    pub cwd: String,
    /// The shell program.
    pub shell: String,
    /// The shell's process id (`None` only if the OS did not report one).
    pub pid: Option<u32>,
    /// Running or exited.
    pub state: TerminalState,
    /// Current columns.
    pub cols: u16,
    /// Current rows.
    pub rows: u16,
    /// How many times it was restarted.
    pub generation: u32,
    /// Offset just past the last output byte (all output ever, not only the scrollback).
    pub output_end: u64,
    /// When it was opened (Unix milliseconds).
    pub started_at_ms: u64,
}

/// A slice of a terminal's output as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRead {
    /// Offset of the first byte returned.
    pub start: u64,
    /// Offset just past the last byte returned; ask from here next time.
    pub end: u64,
    /// True when the requested start was already dropped from the scrollback.
    pub truncated: bool,
    /// The bytes as UTF-8 text (cut on character boundaries; invalid bytes replaced).
    pub text: String,
}

/// What [`crate::TerminalManager::attach`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachInfo {
    /// Identifies this viewer in `ack` and `detach` (a later attach replaces it).
    pub viewer: u64,
    /// Offset of the first replayed byte.
    pub start: u64,
    /// Offset just past the replay; live output follows from here.
    pub end: u64,
}

/// Lifecycle events, published through [`crate::TerminalEvents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// A terminal was opened.
    Opened(TerminalInfo),
    /// A terminal's shell ended (the terminal stays until closed or restarted).
    Exited(TerminalInfo),
    /// A terminal was restarted (new shell, same id and scrollback).
    Restarted(TerminalInfo),
    /// A terminal was closed and removed.
    Closed {
        /// The terminal.
        id: TerminalId,
        /// Its session.
        session_id: String,
    },
}

/// Receives output bytes for an attached viewer. Returns `false` when the viewer is gone (the
/// terminal then detaches it and keeps buffering). Called under the output lock by attach, the
/// reader, or the timed flush worker; must not block or re-enter the manager.
pub type ViewerSink = Box<dyn FnMut(&[u8]) -> bool + Send>;
