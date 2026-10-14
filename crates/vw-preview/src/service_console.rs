//! Current-page console diagnostics for the preview service.
//! Depends on: the preview state and engine drain. Called by: the core's preview and browser tools.

use url::Url;

use super::{PreviewError, PreviewService};
use crate::console::{ConsoleEntry, MAX_RETURNED};

impl PreviewService {
    /// Recent console errors and warnings (at most `limit`, at most [`MAX_RETURNED`]), newest last, and how many
    /// older ones were dropped from the current document's host ring.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`]; a drain failure of a mounted preview is reported, not hidden.
    pub async fn console(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<(Vec<ConsoleEntry>, u64), PreviewError> {
        let _activity = self.session_lock(session_id).await;
        let (mount, epoch) = {
            let state = self.inner.lock();
            let entry = state
                .entries
                .get(session_id)
                .ok_or(PreviewError::NothingOpen)?;
            // A Started event can precede the webview switching documents. Its old page may
            // still answer an evaluation, so do not drain until the new document finishes.
            let mount = if entry.loading || entry.pending_open.is_some() {
                None
            } else {
                entry.mount.clone()
            };
            (mount, entry.console_epoch)
        };
        if let Some(mount) = mount {
            self.touch_pool(session_id);
            let id = self.mount_id(session_id, &mount);
            let drained = self
                .bounded("console", self.engine.drain_console(&id))
                .await?;
            let mut state = self.inner.lock();
            let entry = state.entry(session_id);
            if entry.console_epoch == epoch
                && entry.mount.as_ref().map(|m| &m.label) == Some(&mount.label)
            {
                let origin = entry.url.as_ref().map(Url::origin);
                entry.push_console(
                    drained
                        .into_iter()
                        .filter(|item| {
                            origin.as_ref().is_some_and(|current| {
                                Url::parse(&item.url).is_ok_and(|url| url.origin() == *current)
                            })
                        })
                        .collect(),
                );
            }
        }
        let state = self.inner.lock();
        let entry = state
            .entries
            .get(session_id)
            .ok_or(PreviewError::NothingOpen)?;
        let take = limit.unwrap_or(MAX_RETURNED).min(MAX_RETURNED);
        let skip = entry.console.len().saturating_sub(take);
        Ok((
            entry.console.iter().skip(skip).cloned().collect(),
            entry.console_dropped,
        ))
    }
}
