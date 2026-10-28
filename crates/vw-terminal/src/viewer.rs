//! Batches one viewer's live bytes, preserving order and immediate echo after idle.
//! Depends on `types`, `viewer_stats`, std time. Called under `flow`'s scrollback lock.

use std::time::{Duration, Instant};

use crate::types::ViewerSink;
use crate::viewer_stats::ViewerStats;

/// Largest message, including replay; bounds the work handed to the page at once.
pub(crate) const MESSAGE_CAP: usize = 64 * 1024;
/// A trailing batch is due this long after its first byte, regardless of subsequent reads.
pub(crate) const FLUSH_WINDOW: Duration = Duration::from_millis(8);

pub(crate) struct Viewer {
    pub(crate) id: u64,
    /// Sent bytes awaiting acknowledgement. Pending bytes also count towards the high water.
    pub(crate) unacked: u64,
    pub(crate) deadline: Option<Instant>,
    sink: ViewerSink,
    pending: Vec<u8>,
    last_sent: Option<Instant>,
    stats: Option<ViewerStats>,
}

impl Viewer {
    pub(crate) fn new(id: u64, sink: ViewerSink) -> Self {
        Self {
            id,
            sink,
            unacked: 0,
            deadline: None,
            pending: Vec::new(),
            last_sent: None,
            stats: ViewerStats::enabled(id),
        }
    }

    pub(crate) fn outstanding(&self) -> u64 {
        self.unacked + self.pending.len() as u64
    }

    pub(crate) fn report(&self, output_end: u64) {
        if let Some(stats) = &self.stats {
            stats.report(Some(output_end));
        }
    }

    fn sent(&mut self, bytes: usize, now: Instant) {
        self.unacked += bytes as u64;
        self.last_sent = Some(now);
        ViewerStats::record(&mut self.stats, bytes);
    }

    /// Immediate output (replay or app markers), split at the same cap as live output.
    pub(crate) fn send(&mut self, data: &[u8], now: Instant) -> bool {
        for chunk in data.chunks(MESSAGE_CAP) {
            self.sent(chunk.len(), now);
            if !(self.sink)(chunk) {
                return false;
            }
        }
        true
    }

    /// The leading read after idle goes straight through. Following reads share a bounded batch.
    pub(crate) fn push(&mut self, mut data: &[u8], now: Instant) -> bool {
        while !data.is_empty() {
            if self.pending.is_empty()
                && self
                    .last_sent
                    .is_none_or(|sent| now.duration_since(sent) >= FLUSH_WINDOW)
            {
                let n = data.len().min(MESSAGE_CAP);
                if !self.send(&data[..n], now) {
                    return false;
                }
                data = &data[n..];
                continue;
            }
            if self.pending.is_empty() {
                self.deadline = Some(now + FLUSH_WINDOW);
            }
            let n = data.len().min(MESSAGE_CAP - self.pending.len());
            self.pending.extend_from_slice(&data[..n]);
            data = &data[n..];
            if self.pending.len() == MESSAGE_CAP && !self.flush(now) {
                return false;
            }
        }
        true
    }

    pub(crate) fn flush(&mut self, now: Instant) -> bool {
        self.deadline = None;
        if self.pending.is_empty() {
            return true;
        }
        self.sent(self.pending.len(), now);
        let alive = (self.sink)(&self.pending);
        self.pending.clear();
        alive
    }
}
