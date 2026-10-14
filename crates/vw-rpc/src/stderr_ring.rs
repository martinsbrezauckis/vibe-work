//! Bounded ring buffer for the server's stderr, kept for diagnostics only.
//!
//! Depends on: `framing` (bounded line reads). Called by: the connection (pump task, death
//! reports) and the supervisor (diagnostics).
//! Invariants: stderr is never parsed as protocol and never logged; the ring holds at most
//! `max_lines` lines and `max_bytes` bytes, each line at most `max_line_bytes` (cut on a UTF-8
//! boundary with a marker); every configured secret value is replaced by [`REDACTED`] before a
//! line is stored (redaction runs before truncation, so a cut can never expose part of a key).
//!
//! Adapted from ShellX `src-tauri/src/process_registry.rs:262-292 @ e38c2b5c` (the line + byte
//! capped tail and `bounded_process_line`); the registry around it is not ported.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncRead, BufReader};

use crate::framing::{BoundedLine, read_bounded_line};

/// Appended to a stderr line that was cut to the per-line limit.
pub const TRUNCATION_MARKER: &str = " [truncated]";

/// Replaces a redacted secret in captured stderr.
pub const REDACTED: &str = "<redacted>";

/// Exact secret values to remove from captured server output (e.g. the Mistral API key).
/// `Debug` shows only how many values it holds, never the values.
#[derive(Clone, Default)]
pub struct Redactions(Arc<Vec<String>>);

impl Redactions {
    /// Redactions for the given values. Empty strings are ignored (they would match everywhere).
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = String>) -> Self {
        Self(Arc::new(
            values.into_iter().filter(|v| !v.is_empty()).collect(),
        ))
    }

    /// These redactions plus `values` (empty strings ignored).
    #[must_use]
    pub fn extended(&self, values: impl IntoIterator<Item = String>) -> Self {
        let mut all: Vec<String> = self.0.iter().cloned().collect();
        all.extend(values.into_iter().filter(|v| !v.is_empty()));
        Self(Arc::new(all))
    }

    /// Number of secret values held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when no value is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `text` with every configured value replaced by [`REDACTED`].
    #[must_use]
    pub fn apply(&self, mut text: String) -> String {
        for secret in self.0.iter() {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), REDACTED);
            }
        }
        text
    }
}

impl fmt::Debug for Redactions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Redactions({} values)", self.0.len())
    }
}

/// Limits for a [`StderrRing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StderrLimits {
    /// Most lines kept (oldest dropped first).
    pub max_lines: usize,
    /// Most bytes kept across all lines.
    pub max_bytes: usize,
    /// Longest single line kept; longer lines are truncated.
    pub max_line_bytes: usize,
}

impl Default for StderrLimits {
    /// 200 lines, 256 KiB total, 16 KiB per line: enough for a Python traceback.
    fn default() -> Self {
        Self {
            max_lines: 200,
            max_bytes: 256 * 1024,
            max_line_bytes: 16 * 1024,
        }
    }
}

#[derive(Debug, Default)]
struct Ring {
    lines: VecDeque<String>,
    bytes: usize,
}

/// Thread-safe bounded tail of stderr lines.
#[derive(Debug)]
pub struct StderrRing {
    limits: StderrLimits,
    redactions: Redactions,
    ring: Mutex<Ring>,
    total_lines: AtomicU64,
}

impl StderrRing {
    /// Creates an empty ring with the given limits and no redactions.
    #[must_use]
    pub fn new(limits: StderrLimits) -> Self {
        Self::with_redactions(limits, Redactions::default())
    }

    /// Creates an empty ring that removes the given secret values from every line.
    #[must_use]
    pub fn with_redactions(limits: StderrLimits, redactions: Redactions) -> Self {
        Self {
            limits,
            redactions,
            ring: Mutex::new(Ring::default()),
            total_lines: AtomicU64::new(0),
        }
    }

    /// Appends one line (secrets redacted, then truncated to the per-line limit), evicting old
    /// lines as needed.
    pub fn push(&self, line: String) {
        let line = truncate_line(self.redactions.apply(line), self.limits.max_line_bytes);
        self.total_lines.fetch_add(1, Ordering::Relaxed);
        if self.limits.max_lines == 0 || line.len() > self.limits.max_bytes {
            // Cannot be kept at all (only with limits where one line exceeds the whole budget);
            // check before evicting so good lines are not thrown away for it.
            return;
        }
        let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        while ring.lines.len() >= self.limits.max_lines
            || ring.bytes.saturating_add(line.len()) > self.limits.max_bytes
        {
            let Some(old) = ring.lines.pop_front() else {
                break;
            };
            ring.bytes = ring.bytes.saturating_sub(old.len());
        }
        ring.bytes = ring.bytes.saturating_add(line.len());
        ring.lines.push_back(line);
    }

    /// The last `n` kept lines, oldest first.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let skip = ring.lines.len().saturating_sub(n);
        ring.lines.iter().skip(skip).cloned().collect()
    }

    /// Number of lines ever pushed (including evicted ones).
    #[must_use]
    pub fn total_lines(&self) -> u64 {
        self.total_lines.load(Ordering::Relaxed)
    }

    /// Bytes currently held.
    #[must_use]
    pub fn held_bytes(&self) -> usize {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .bytes
    }
}

/// Cuts `line` to at most `max` bytes on a char boundary, marking the cut.
fn truncate_line(mut line: String, max: usize) -> String {
    if line.len() <= max {
        return line;
    }
    let mut cut = max.saturating_sub(TRUNCATION_MARKER.len()).min(line.len());
    while cut > 0 && !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    line.push_str(TRUNCATION_MARKER);
    line
}

/// Reads stderr until end of stream, pushing every line into `ring`.
///
/// Lines over the per-line read limit (4 x `max_line_bytes`) are dropped with a placeholder so a
/// newline-free stderr flood cannot grow memory. Invalid UTF-8 is replaced, never rejected.
/// Returns when the stream ends or fails (a read error ends the pump silently: stderr is
/// diagnostics, and stdout EOF is what declares the server dead).
pub(crate) async fn pump<R>(reader: R, ring: &StderrRing)
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let read_limit = ring.limits.max_line_bytes.saturating_mul(4);
    loop {
        match read_bounded_line(&mut reader, read_limit).await {
            Ok(BoundedLine::Line(bytes)) => ring.push(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(BoundedLine::Overflow { discarded }) => {
                ring.push(format!("[stderr line of {discarded} bytes dropped]"));
            }
            Ok(BoundedLine::Eof) | Err(_) => return,
        }
    }
}
