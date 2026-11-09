//! The one host overlay admission path for completed built-in input. Uses the shared action tracker, exact
//! mount/document identity and the engine's visibility check; never remounts, captures, focuses or changes a page.
use crate::action_tracking::{
    ActionGeometry, ActionKind, ActionSource, ActionTracker, SourceBrowser,
};
use crate::{MountId, PreviewService};
use std::sync::{Arc, Once};

static MARKER_ERROR: Once = Once::new();

fn report_marker_result(once: &Once, result: Result<(), crate::EngineError>) {
    if let Err(error) = result {
        once.call_once(|| tracing::warn!(%error, "could not draw browser input marker"));
    }
}

impl PreviewService {
    /// The same timeline receives external Chrome/Edge receipts at the core boundary.
    #[must_use]
    pub fn action_tracker(&self) -> &Arc<ActionTracker> {
        &self.actions
    }

    /// Track a confirmed agent input and paint only on its still-current native surface. Failure is best effort.
    pub async fn record_input(
        &self,
        id: &MountId,
        document: &str,
        loads: u64,
        kind: ActionKind,
        geometry: ActionGeometry,
    ) {
        let Some(session_id) = self.owner_session_id(&id.session_id) else {
            return;
        };
        let source = ActionSource {
            browser: SourceBrowser::BuiltIn,
            view_id: Some(id.label.clone()),
            document_id: Some(document.into()),
            profile_id: None,
            browser_session_id: None,
        };
        let event = self
            .actions
            .record(session_id.clone(), kind, source.clone(), geometry);
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id)
            || self
                .load_state(&id.session_id)
                .is_none_or(|state| state.loading || state.loads != loads)
        {
            return;
        }
        if let Some(marker) = self.actions.marker_for(&event, &session_id, &source) {
            let result = self
                .bounded(
                    "showing browser input",
                    self.engine.show_input_marker(id, marker),
                )
                .await;
            report_marker_result(&MARKER_ERROR, result);
        }
    }
}

#[cfg(test)]
#[path = "action_overlay_tests.rs"]
mod tests;
