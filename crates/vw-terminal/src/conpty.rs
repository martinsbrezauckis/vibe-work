//! The Windows pseudo console's startup handshake.
//!
//! portable-pty 0.9 creates every pseudo console with `PSEUDOCONSOLE_INHERIT_CURSOR`
//! (`win/psuedocon.rs:83-90`, not configurable). With that flag the console host first writes a
//! cursor-position query (`ESC [ 6 n`) and **renders nothing until the terminal answers** with
//! `ESC [ row ; col R` (Microsoft's `CreatePseudoConsole` documentation; observed on Windows 11:
//! 4 bytes of output, then silence, until the reply was written, then the PowerShell prompt).
//!
//! The core answers that one query itself, for three reasons: a terminal opened through the Debug
//! API, or in a hidden session, has no view that could answer; the view attaches after the shell
//! started; and a query left in the scrollback would be answered again by every view that attaches
//! later, typing `ESC[1;1R` into the shell. So the query is removed from the output and answered
//! with row 1, column 1: a new terminal's screen is empty, and a restart first moves the old lines
//! into the scrollback and the cursor home (`restart_preamble`).
//!
//! Depends on: nothing. Called by: `terminal` (reader thread, Windows only; compiled everywhere so
//! the private tests can check it on Linux).
//! Invariants: only the first query of a shell generation is answered, and only when it arrives in
//! that generation's first `WINDOW` bytes of output; later queries (from programs) pass through to
//! the view untouched.

/// The cursor-position query.
pub const QUERY: &[u8] = b"\x1b[6n";
/// The answer: row 1, column 1.
pub const REPLY: &[u8] = b"\x1b[1;1R";
/// How far into a generation's output the startup query is looked for.
const WINDOW: usize = 256;

/// Tracks one shell generation's startup handshake.
#[derive(Debug, Default)]
pub struct StartupQuery {
    seen: usize,
    done: bool,
}

impl StartupQuery {
    /// A handshake not yet seen.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters one chunk of output: returns the bytes to pass on and whether the core must write
    /// [`REPLY`] to the console's input now. The query is assumed to arrive in one read (the console
    /// host writes it in one write).
    #[must_use]
    pub fn filter<'a>(&mut self, chunk: &'a [u8]) -> (std::borrow::Cow<'a, [u8]>, bool) {
        if self.done {
            return (chunk.into(), false);
        }
        let room = WINDOW.saturating_sub(self.seen);
        let found = chunk
            .windows(QUERY.len())
            .take(room)
            .position(|w| w == QUERY);
        self.seen += chunk.len();
        let Some(at) = found else {
            self.done = self.seen >= WINDOW;
            return (chunk.into(), false);
        };
        self.done = true;
        let mut kept = Vec::with_capacity(chunk.len() - QUERY.len());
        kept.extend_from_slice(&chunk[..at]);
        kept.extend_from_slice(&chunk[at + QUERY.len()..]);
        (kept.into(), true)
    }
}

/// What the view gets before a restarted shell's first output: enough line breaks to move the old
/// screen into the scrollback, then the cursor home, so it matches the row 1, column 1 answer.
#[must_use]
pub fn restart_preamble(rows: u16) -> Vec<u8> {
    let mut out = b"\r\n".repeat(usize::from(rows));
    out.extend_from_slice(b"\x1b[H");
    out
}
