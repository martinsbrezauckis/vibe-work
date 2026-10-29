//! Main-document generations for native placements: a window command placed by an earlier main document than the
//! last placement reset must not show a native child over the replacement document.
//!
//! Depends on: tokio (task-local). Called by: the host (scopes each main-window command with its generation), the
//! Browser service and the Project/File viewer service (check before a placement, advance in their reset).
//! Invariants:
//! - A placement without a window generation (the core's own work, the Debug API, the agent) is not limited here.
//! - A service checks and advances its gate while holding its own layout lock, so a placement it admits finishes
//!   before the reset hides it, and one it reaches later is refused.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

tokio::task_local! {
    static WINDOW_GENERATION: u64;
}

/// Runs `command` as a main-window command issued by main-document load `generation`.
pub async fn for_window_generation<F: Future>(generation: u64, command: F) -> F::Output {
    WINDOW_GENERATION.scope(generation, command).await
}

/// The latest main-document generation whose placement reset a service has run.
#[derive(Debug, Default)]
pub struct PlacementGate {
    reset: AtomicU64,
}

impl PlacementGate {
    /// Whether the running command may place native children: false only for a window command of a main document
    /// older than the last reset. The caller holds its layout lock.
    #[must_use]
    pub fn admits(&self) -> bool {
        let reset = self.reset.load(Ordering::SeqCst);
        WINDOW_GENERATION
            .try_with(|generation| *generation >= reset)
            .unwrap_or(true)
    }

    /// Marks placements of documents before `generation` as stale. The caller holds its layout lock.
    pub fn advance(&self, generation: u64) {
        self.reset.fetch_max(generation, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_window_commands_of_an_older_document_are_refused() {
        let gate = PlacementGate::default();
        gate.advance(2);
        assert!(
            gate.admits(),
            "work outside a window command is not limited"
        );
        assert!(!for_window_generation(1, async { gate.admits() }).await);
        assert!(for_window_generation(2, async { gate.admits() }).await);
        gate.advance(1);
        assert!(
            !for_window_generation(1, async { gate.admits() }).await,
            "a late reset never reopens an older document"
        );
    }
}
