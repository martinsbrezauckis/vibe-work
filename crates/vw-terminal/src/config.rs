//! Tuning of the terminal manager. `Default` gives production values.
//!
//! Depends on: `shell` (`ShellCommand`). Called by: `manager`, `flow`, `terminal`, the core (from its
//! own config), tests (small values to exercise the limits).

use std::ffi::OsString;
use std::time::Duration;

use crate::shell::ShellCommand;

/// Where a terminal's inherited environment comes from (before `terminal_env` filters it).
#[derive(Debug, Clone, Default)]
pub enum ParentEnv {
    /// This process's environment at the moment the terminal opens.
    #[default]
    Process,
    /// A fixed list (tests: a hostile parent environment without changing the test process).
    Fixed(Vec<(OsString, OsString)>),
}

impl ParentEnv {
    /// The variables to filter.
    #[must_use]
    pub fn vars(&self) -> Vec<(OsString, OsString)> {
        match self {
            Self::Process => std::env::vars_os().collect(),
            Self::Fixed(vars) => vars.clone(),
        }
    }
}

/// Terminal manager settings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TerminalConfig {
    /// Most terminals one session may have open at once (tabs). Default 4.
    pub max_per_session: usize,
    /// Scrollback kept per terminal, in bytes; the oldest output is dropped first. Default 2 MiB.
    pub scrollback_bytes: usize,
    /// Unacknowledged bytes an attached viewer may have before the terminal stops reading its
    /// program's output (the program then blocks on its next write). Default 1 MiB.
    pub high_water: u64,
    /// How long a terminal waits for a viewer's acknowledgement before it detaches that viewer and
    /// goes back to buffering (a reloaded or frozen window must not stop the program forever).
    /// Default 10 s.
    pub ack_timeout: Duration,
    /// Time between the hang-up signal and the hard kill when a terminal closes. Default 300 ms.
    pub hangup_grace: Duration,
    /// Longest wait for a closing terminal's process tree to be gone. Default 5 s (ShellX
    /// `terminal.rs:66-70`: generous for `ConPTY` teardown, still bounded).
    pub close_timeout: Duration,
    /// Keystrokes and pastes waiting for the PTY (chunks); a full queue refuses input instead of
    /// blocking. Default 256.
    pub input_queue: usize,
    /// The shell to start; `None` picks the user's default ([`crate::default_shell`]).
    pub shell: Option<ShellCommand>,
    /// Where the inherited environment comes from.
    pub parent_env: ParentEnv,
    /// App-supplied non-secret routing for agent-capable terminals, after parent filtering.
    pub extra_env: Vec<(OsString, OsString)>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            max_per_session: 4,
            scrollback_bytes: 2 * 1024 * 1024,
            high_water: 1024 * 1024,
            ack_timeout: Duration::from_secs(10),
            hangup_grace: Duration::from_millis(300),
            close_timeout: Duration::from_secs(5),
            input_queue: 256,
            shell: None,
            parent_env: ParentEnv::Process,
            extra_env: Vec::new(),
        }
    }
}
