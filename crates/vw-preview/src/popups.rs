//! Dispatches native new-window requests to the host's managed-tab handler.
//! Depends on current mount labels; the core owns chat authority and navigation policy.
use crate::{PreviewHooks, PreviewService, hooks::Hooks};
use std::sync::Arc;
use url::Url;

impl PreviewService {
    /// Installs the host's asynchronous managed-tab route. The callback returns whether it queued the request.
    pub fn on_new_window(
        &self,
        handler: impl Fn(crate::MountId, Url) -> bool + Send + Sync + 'static,
    ) {
        *self
            .inner
            .popup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(handler));
    }
}

impl Hooks {
    pub(crate) fn route_popup(&self, label: &str, url: &Url) {
        let source = {
            let state = self.0.lock();
            state.session_of(label).map(|page| crate::MountId {
                session_id: page.to_owned(),
                label: label.to_owned(),
            })
        };
        let handler = self
            .0
            .popup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let (Some(source), Some(handler)) = (source, handler)
            && handler(source, url.clone())
        {
            return;
        }
        self.new_window_refused(label, url);
    }
}
