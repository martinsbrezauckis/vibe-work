//! Mounted-page script admission. Depends on the service's page lock and the caller's host authority callback.
//! Browser dispatch and ordinary preview reads share this route; released mounts are never recreated.

use std::sync::Arc;

use crate::{EngineError, MountId, PreviewError, PreviewService};

impl PreviewService {
    /// Evaluates a script only while `id` is the session's mounted preview.
    ///
    /// # Errors
    /// Closed/replaced mounts, engine failures and timeouts.
    pub async fn evaluate_on_mount(
        &self,
        id: &MountId,
        script: String,
    ) -> Result<String, PreviewError> {
        self.evaluate_on_mount_checked(id, script, None).await
    }

    /// Rechecks host admission after acquiring the page lock, immediately before queueing evaluation.
    /// The caller serializes permission writes until the evaluation reply; pages cannot supply this callback.
    ///
    /// # Errors
    /// Closed/replaced mounts, withdrawn permission or page ownership, engine failures and timeouts.
    pub async fn evaluate_on_mount_checked(
        &self,
        id: &MountId,
        script: String,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<String, PreviewError> {
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id) {
            return Err(EngineError::NotMounted.into());
        }
        if allowed.is_some_and(|allowed| !allowed()) {
            return Err(EngineError::Platform(
                "Not run: browser permission changed or page ownership ended before input."
                    .to_owned(),
            )
            .into());
        }
        self.touch_pool(&id.session_id);
        Ok(self
            .bounded("evaluate", self.engine.evaluate(id, script))
            .await?)
    }
}
