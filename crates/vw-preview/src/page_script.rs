//! Main-world execution for explicitly authorized page scripts, isolated from the agent's trusted world.
//! The page lock and atomic vault exclusion admit dispatch once; later polls never repeat the expression.
use crate::{EngineError, MountId, PreviewError, PreviewService};
use std::sync::Arc;
impl PreviewService {
    /// Dispatches or polls page-world code on the same live document, with vault exclusion.
    ///
    /// # Errors
    /// Changed document, withdrawn permission, vault fill, native timeout or engine error.
    pub async fn page_script(
        &self,
        id: &MountId,
        loads: u64,
        script: String,
        dispatch: bool,
        allowed: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Result<String, PreviewError> {
        let _activity = self.session_lock(&id.session_id).await;
        if self.page_mount(&id.session_id).as_ref() != Some(id)
            || self
                .load_state(&id.session_id)
                .is_none_or(|state| state.loading || state.loads != loads)
        {
            return Err(EngineError::Platform("The document changed before the script result was available; read the page before continuing, and do not repeat side effects automatically.".into()).into());
        }
        if allowed.is_some_and(|check| !check()) {
            return Err(EngineError::Platform("Browser permission changed or the page stopped; read the page after access is restored.".into()).into());
        }
        {
            let mut state = self.inner.lock();
            let entry = state
                .entries
                .get_mut(&id.session_id)
                .ok_or(PreviewError::NothingOpen)?;
            if entry.shots_blocked_at == Some(loads) {
                return Err(EngineError::Platform("Page scripts are blocked after a vault fill because they could read the password; reload or navigate to a new document before running JavaScript.".into()).into());
            }
            if dispatch {
                entry.scripted_at = Some(loads);
            }
        }
        self.touch_pool(&id.session_id);
        Ok(self
            .bounded("page script", self.engine.evaluate(id, script))
            .await?)
    }

    /// Whether the current document has received page code that could observe a later vault fill.
    #[must_use]
    pub fn page_was_scripted(&self, session: &str) -> bool {
        self.inner
            .lock()
            .entries
            .get(session)
            .is_some_and(|entry| entry.scripted_at == Some(*entry.loads.borrow()))
    }
}
