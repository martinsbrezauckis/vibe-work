//! The terminal part of the command API: the requests both clients send inside
//! `Command::Terminal`, what comes back, and the terminal core event.
//!
//! Depends on: vw-terminal (the types these views are built from), serde. Called by: `command`,
//! `event`, `terminals` (the handler), the UI through the generated TypeScript, vw-debug-api.
//! Invariants:
//! - Wire form: `{"op": "<camelCase>", ...camelCase fields}`, unknown fields refused.
//! - Nothing here carries terminal input back out: results describe terminals and output only.
//! - Offsets and timestamps are JSON numbers (exact up to 2^53, far beyond any terminal's output).

use serde::{Deserialize, Serialize};
use vw_terminal::{OutputRead, TerminalEvent, TerminalInfo, TerminalState};

/// One request about terminals. The Debug API acts with the user's rights; writes from it and
/// the app window run directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TerminalRequest {
    /// Opens a terminal in the session's folder (as the core knows it).
    Open {
        /// The session.
        session_id: String,
        /// Initial columns (default 80).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cols: Option<u16>,
        /// Initial rows (default 24).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        rows: Option<u16>,
    },
    /// The terminals of one session, or of all sessions.
    List {
        /// The session (all when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        session_id: Option<String>,
    },
    /// Sends input (keystrokes, a paste, a command line with its newline).
    Write {
        /// The terminal.
        terminal_id: String,
        /// The input text. With `binary`, each character (U+0000..U+00FF) is one byte (xterm's
        /// `onBinary` mouse reports).
        data: String,
        /// See `data`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        binary: Option<bool>,
    },
    /// Resizes the terminal (2..=1000 columns, 1..=500 rows).
    Resize {
        /// The terminal.
        terminal_id: String,
        /// Columns.
        cols: u16,
        /// Rows.
        rows: u16,
    },
    /// Reads output as text (the Debug API tail; the browser dev client's stream).
    Read {
        /// The terminal.
        terminal_id: String,
        /// Start offset; absent: the newest `maxBytes`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
        from: Option<u64>,
        /// Most bytes to read (default 64 KiB, at most 1 MiB).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
        max_bytes: Option<u64>,
        /// Remove escape sequences (colours, titles, links, clipboard requests).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        plain: Option<bool>,
    },
    /// Stops the shell and its processes and starts a new shell (same id and scrollback).
    Restart {
        /// The terminal.
        terminal_id: String,
    },
    /// Stops the shell and every process it started, and removes the terminal.
    Close {
        /// The terminal.
        terminal_id: String,
    },
}

/// What a terminal request returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalResult {
    /// `open` and `restart`: the terminal now.
    Terminal {
        /// The terminal.
        terminal: TerminalView,
    },
    /// `list`.
    Terminals {
        /// Oldest first.
        terminals: Vec<TerminalView>,
    },
    /// `read`.
    Output {
        /// The text read.
        output: TerminalOutputView,
    },
    /// `write`, `resize`, `close`.
    Done {},
}

/// Running or exited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalStateView {
    /// The shell runs.
    Running {},
    /// The shell ended.
    Exited {
        /// Exit code (Unix: 1 when killed by a signal).
        exit_code: Option<u32>,
        /// Signal name (Unix).
        signal: Option<String>,
    },
}

/// One terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TerminalView {
    /// Its id.
    pub terminal_id: String,
    /// Its session.
    pub session_id: String,
    /// The folder it started in.
    pub cwd: String,
    /// The shell program.
    pub shell: String,
    /// The shell's process id.
    pub pid: Option<u32>,
    /// Running or exited.
    pub state: TerminalStateView,
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
    /// Restarts so far.
    pub generation: u32,
    /// Offset just past the newest output byte.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub output_end: u64,
    /// When it was opened (Unix milliseconds).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub started_at_ms: u64,
}

impl From<TerminalInfo> for TerminalView {
    fn from(info: TerminalInfo) -> Self {
        Self {
            terminal_id: info.id.to_string(),
            session_id: info.session_id,
            cwd: info.cwd,
            shell: info.shell,
            pid: info.pid,
            state: match info.state {
                TerminalState::Running => TerminalStateView::Running {},
                TerminalState::Exited(exit) => TerminalStateView::Exited {
                    exit_code: exit.exit_code,
                    signal: exit.signal,
                },
            },
            cols: info.cols,
            rows: info.rows,
            generation: info.generation,
            output_end: info.output_end,
            started_at_ms: info.started_at_ms,
        }
    }
}

/// Output read as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputView {
    /// Offset of the first byte returned.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub start: u64,
    /// Offset just past the last byte returned; ask `from` here next time.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub end: u64,
    /// The requested start was already dropped from the scrollback.
    pub truncated: bool,
    /// The text.
    pub text: String,
}

impl From<OutputRead> for TerminalOutputView {
    fn from(read: OutputRead) -> Self {
        Self {
            start: read.start,
            end: read.end,
            truncated: read.truncated,
            text: read.text,
        }
    }
}

/// A terminal lifecycle change (inside `CoreEvent::Terminal`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEventView {
    /// Opened.
    Opened {
        /// The terminal.
        terminal: TerminalView,
    },
    /// The shell exited (the terminal stays until closed or restarted).
    Exited {
        /// The terminal.
        terminal: TerminalView,
    },
    /// Restarted with a new shell.
    Restarted {
        /// The terminal.
        terminal: TerminalView,
    },
    /// Closed and removed.
    Closed {
        /// The terminal.
        terminal_id: String,
        /// Its session.
        session_id: String,
    },
}

impl TerminalEventView {
    /// The session the terminal belongs to.
    #[must_use]
    pub fn session_id(&self) -> &str {
        match self {
            Self::Opened { terminal }
            | Self::Exited { terminal }
            | Self::Restarted { terminal } => &terminal.session_id,
            Self::Closed { session_id, .. } => session_id,
        }
    }
}

impl From<TerminalEvent> for TerminalEventView {
    fn from(event: TerminalEvent) -> Self {
        match event {
            TerminalEvent::Opened(info) => Self::Opened {
                terminal: info.into(),
            },
            TerminalEvent::Exited(info) => Self::Exited {
                terminal: info.into(),
            },
            TerminalEvent::Restarted(info) => Self::Restarted {
                terminal: info.into(),
            },
            TerminalEvent::Closed { id, session_id } => Self::Closed {
                terminal_id: id.to_string(),
                session_id,
            },
        }
    }
}
