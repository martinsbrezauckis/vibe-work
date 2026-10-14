//! Applies a shown preview layout after the service records the desired session.
//! Depends on the preview engine, placement model and mount state; called by `PreviewService::layout`.

use super::{PreviewError, PreviewService};
use crate::layout;
use crate::state::BrowserMountState;

/// Native-only placement preserves panel ownership; a panel report replaces or clears it.
pub(super) enum PanelSelection {
    Unchanged,
    Set(Option<String>),
}

impl PreviewService {
    pub(super) async fn apply_layout_target(&self, session: &str) -> Result<(), PreviewError> {
        let session = session.to_owned();
        let _activity = self.session_lock(&session).await;
        let current = {
            let state = self.inner.lock();
            state.entries.get(&session).and_then(|entry| {
                let shown = state.shown.as_ref()?;
                let placement = layout::place(shown.area, entry.width).ok()?;
                Some((
                    entry.mount.clone(),
                    entry.url.clone(),
                    placement,
                    entry.server_status.is_some(),
                ))
            })
        };
        let Some((mount, url, placement, stopped)) = current else {
            return Ok(());
        };
        if stopped {
            if let Some(mount) = mount {
                let id = self.mount_id(&session, &mount);
                self.bounded("hide", self.engine.place(&id, None)).await?;
            }
            return Ok(());
        }
        match (mount, url) {
            (Some(mount), _) => {
                self.touch_pool(&session);
                let id = self.mount_id(&session, &mount);
                self.bounded("place", self.engine.place(&id, Some(placement)))
                    .await?;
            }
            (None, Some(url)) => {
                let held = self
                    .inner
                    .lock()
                    .entries
                    .get(&session)
                    .is_some_and(|entry| entry.browser_mount != BrowserMountState::Ready);
                if held {
                    return Ok(());
                }
                self.mount(&session, url, Some(placement)).await?;
                self.enforce_cap(&session).await;
            }
            (None, None) => {}
        }
        Ok(())
    }
}
