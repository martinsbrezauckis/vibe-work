//! Current-page console diagnostics for the preview service.
//! Depends on: the preview state and engine drain. Called by: the core's preview and browser tools, and the
//! suspension path (`service_mount::suspend_in_pool` reads the page before its webview closes).

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
        self.drain_to_host(session_id, true).await?;
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

    /// Moves what the current document logged from the page's ring into the host ring (only entries of the
    /// document's origin). Nothing is read while a document is loading or the preview has no ready mount: the
    /// host ring then already holds what was read before (a suspension reads the page before it closes, so a
    /// suspended preview keeps its console). `touch` marks the engine as used (a read the agent asked for; a
    /// suspension does not, it is about to give the engine up). The caller holds the session's operation lock.
    ///
    /// # Errors
    /// [`PreviewError::NothingOpen`]; a drain failure of a mounted preview.
    pub(super) async fn drain_to_host(
        &self,
        session_id: &str,
        touch: bool,
    ) -> Result<(), PreviewError> {
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
                entry.ready_mount().cloned()
            };
            (mount, entry.console_epoch)
        };
        let Some(mount) = mount else {
            return Ok(());
        };
        if touch {
            self.touch_pool(session_id);
        }
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
        Ok(())
    }
}
