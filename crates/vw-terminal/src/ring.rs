//! The scrollback: the newest `capacity` bytes of a terminal's output, addressed by monotonic
//! offsets (offset 0 is the first byte the terminal ever printed).
//!
//! Depends on: std. Called by: `flow` (append, replay, reads), tests.
//! Invariants:
//! - `start() <= end()` and `end() - start() <= capacity`; `end()` only grows.
//! - Eviction drops whole bytes from the front only; what remains is exactly the bytes
//!   `start()..end()` in order (chunks are a storage detail).
//! - Memory is bounded by `capacity` plus one chunk header per stored chunk (chunks are at most
//!   one PTY read, 64 KiB).
//!
//! ShellX kept 64 KiB in a `VecDeque<u8>` and evicted byte by byte (`terminal.rs:139-163`); here the
//! store is chunked and the cap is configurable.

use std::collections::VecDeque;

/// A bounded byte ring with absolute offsets.
#[derive(Debug, Clone)]
pub struct Ring {
    chunks: VecDeque<Vec<u8>>,
    /// Offset of the first stored byte.
    start: u64,
    /// Bytes stored.
    len: usize,
    capacity: usize,
}

impl Ring {
    /// An empty ring keeping at most `capacity` bytes (at least 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            start: 0,
            len: 0,
            capacity: capacity.max(1),
        }
    }

    /// Offset of the oldest byte still stored.
    #[must_use]
    pub fn start(&self) -> u64 {
        self.start
    }

    /// Offset just past the newest byte.
    #[must_use]
    pub fn end(&self) -> u64 {
        self.start + self.len as u64
    }

    /// Bytes stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Appends `data`, dropping the oldest bytes beyond the capacity.
    pub fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data.len() >= self.capacity {
            // Only the tail of this chunk survives; everything older goes.
            let skipped = data.len() - self.capacity;
            let new_start = self.end() + skipped as u64;
            self.chunks.clear();
            self.chunks.push_back(data[skipped..].to_vec());
            self.start = new_start;
            self.len = self.capacity;
            return;
        }
        self.chunks.push_back(data.to_vec());
        self.len += data.len();
        let mut excess = self.len.saturating_sub(self.capacity);
        while excess > 0 {
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            if front.len() <= excess {
                excess -= front.len();
                self.len -= front.len();
                self.start += front.len() as u64;
                self.chunks.pop_front();
            } else {
                front.drain(..excess);
                self.len -= excess;
                self.start += excess as u64;
                excess = 0;
            }
        }
    }

    /// Copies up to `max` bytes starting at offset `from` (clamped into `start()..=end()`).
    /// Returns the offset actually used and the bytes.
    #[must_use]
    pub fn read(&self, from: u64, max: usize) -> (u64, Vec<u8>) {
        let from = from.clamp(self.start, self.end());
        let mut skip = usize::try_from(from - self.start).unwrap_or(usize::MAX);
        let mut out = Vec::with_capacity(max.min(self.len));
        for chunk in &self.chunks {
            if out.len() >= max {
                break;
            }
            if skip >= chunk.len() {
                skip -= chunk.len();
                continue;
            }
            let take = (chunk.len() - skip).min(max - out.len());
            out.extend_from_slice(&chunk[skip..skip + take]);
            skip = 0;
        }
        (from, out)
    }
}
