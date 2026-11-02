//! Paired clock readings: epoch dates for views and monotonic instants for in-memory spans.
//! Depends on Tokio time and the changes timestamp helper; used by Git reads and install offers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClockReading {
    pub(crate) wall_ms: u64,
    pub(crate) instant: tokio::time::Instant,
}

impl From<u64> for ClockReading {
    fn from(wall_ms: u64) -> Self {
        Self {
            wall_ms,
            instant: tokio::time::Instant::now(),
        }
    }
}

impl ClockReading {
    pub(crate) fn now() -> Self {
        vw_changes::now_ms().into()
    }
}

#[cfg(test)]
#[path = "span_clock_tests.rs"]
mod tests;
