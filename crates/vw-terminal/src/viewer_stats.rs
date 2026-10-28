//! Optional viewer-message counters for diagnosing terminal throughput, with no output contents.
//! Depends on tracing; called by `viewer`. Disabled counters allocate nothing and read no clocks.

/// Per-attachment counts, emitted once when the viewer is replaced, detached or closed.
pub(crate) struct ViewerStats {
    viewer: u64,
    messages: u64,
    bytes: u64,
    min: usize,
    max: usize,
    buckets: [u64; 7],
}

impl ViewerStats {
    pub(crate) fn enabled(viewer: u64) -> Option<Self> {
        if !tracing::enabled!(tracing::Level::DEBUG) {
            return None;
        }
        Some(Self {
            viewer,
            messages: 0,
            bytes: 0,
            min: usize::MAX,
            max: 0,
            buckets: [0; 7],
        })
    }

    pub(crate) fn record(stats: &mut Option<Self>, bytes: usize) {
        let Some(stats) = stats else { return };
        stats.messages += 1;
        stats.bytes += bytes as u64;
        stats.min = stats.min.min(bytes);
        stats.max = stats.max.max(bytes);
        let bucket = match bytes {
            0..=255 => 0,
            256..=1023 => 1,
            1024..=4095 => 2,
            4096..=16383 => 3,
            16384..=65535 => 4,
            65536 => 5,
            _ => 6,
        };
        stats.buckets[bucket] += 1;
    }

    /// Cumulative counters at an info read; subtract checkpoints to measure a stream interval.
    pub(crate) fn report(&self, output_end: Option<u64>) {
        tracing::debug!(
            ?output_end,
            viewer = self.viewer,
            messages = self.messages,
            bytes = self.bytes,
            min = if self.messages == 0 { 0 } else { self.min },
            max = self.max,
            buckets = ?self.buckets,
            "terminal viewer messages (buckets: <=255, <=1023, <=4095, <=16383, <=65535, 65536, >65536 bytes)"
        );
    }
}

impl Drop for ViewerStats {
    fn drop(&mut self) {
        self.report(None);
    }
}
