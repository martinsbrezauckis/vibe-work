//! Bounded navigation-start grace and pending/refused outcomes for browser action receipts.
//! Depends on preview lifecycle facts; called by the driver before observing the resulting document.

use crate::{ActError, Action, Browser, driver::refused};
use serde_json::{Value, json};
use std::time::Duration;

impl Browser {
    /// Click handlers may schedule navigation after the DOM first becomes quiet.
    pub(crate) async fn await_action_navigation(
        &self,
        session: &str,
        action: &Action,
        loads: u64,
        refusal_mark: u64,
    ) {
        if !matches!(
            action,
            Action::Click { .. } | Action::Type { submit: true, .. }
        ) && !matches!(action, Action::Press { key, .. } if key == "Enter")
        {
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while tokio::time::Instant::now() < deadline {
            if self
                .service
                .load_state(session)
                .is_some_and(|s| s.loading || s.loads != loads)
                || self.service.refusal_mark(session) != refusal_mark
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }

    /// Refusals take precedence over page observation errors; pending loads never claim completion.
    pub(crate) fn navigation_outcome(
        &self,
        session: &str,
        loads_before: u64,
        mark: u64,
        result: &Value,
    ) -> Option<Result<Value, ActError>> {
        let state = self.service.load_state(session);
        // A completed top-level load wins over its blocked frames/popups. Keep those refusals
        // in the receipt diagnostics, as explicit navigation and later reads already do.
        let completed = state
            .as_ref()
            .is_some_and(|s| !s.loading && s.loads > loads_before);
        if !completed && let Some((url, reason)) = self.service.refusals_since(session, mark).last()
        {
            return Some(Err(refused(format!("Not opened: {url}. {reason}"))));
        }
        state.filter(|s| s.loading).map(|_| Ok(json!({
            "Outcome": "still loading: navigation has not finished; read the browser again to check the result",
            "ok": true,
            "loading": true,
            "change": {"summary": "still loading"},
            "result": result,
        })))
    }
}
