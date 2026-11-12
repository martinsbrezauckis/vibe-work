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

/// Host-held proof of the exact popup opener, including its current document and permission lifetime.
#[derive(Clone)]
pub struct PopupOpener {
    page: String,
    fingerprint: String,
    allowed: Arc<dyn Fn() -> bool + Send + Sync>,
}

#[derive(Default)]
pub(crate) struct Tracker {
    links: std::collections::BTreeMap<String, PopupOpener>,
    closed: u64,
    truncated: bool,
    continuation: Option<serde_json::Value>,
}

impl PreviewService {
    /// Captures an opener before navigation can suspend it; only the host can supply the permission guard.
    ///
    /// # Errors
    /// The opener is not this chat's current mounted, agent-controlled document.
    pub fn popup_opener(
        &self,
        owner: &str,
        source: &crate::MountId,
        allowed: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<PopupOpener, crate::PreviewError> {
        let state = self.inner.lock();
        if state
            .page_owners
            .get(&source.session_id)
            .is_none_or(|id| id != owner)
            || state
                .entries
                .get(&source.session_id)
                .and_then(|entry| entry.mount.as_ref())
                .is_none_or(|mount| mount.label != source.label)
            || state
                .tabs
                .get(owner)
                .and_then(|tabs| tabs.owners.get(&source.session_id))
                .is_none_or(|kind| *kind == crate::PageTabOwner::User)
            || !allowed()
        {
            return Err(crate::PreviewError::InvalidTab(
                "The popup opener changed. Read the page before continuing.".into(),
            ));
        }
        Ok(PopupOpener {
            page: source.session_id.clone(),
            fingerprint: crate::tabs::review_fingerprint(&state, owner, &source.session_id)?,
            allowed,
        })
    }

    /// Retains a successfully opened popup only while its tab still belongs to this chat.
    pub fn retain_popup(&self, owner: &str, popup: &str, opener: PopupOpener) {
        let mut state = self.inner.lock();
        if state.page_owners.get(popup).is_none_or(|id| id != owner) {
            return;
        }
        let tracker = &mut state.tabs.entry(owner.into()).or_default().popups;
        if tracker.links.len() >= 128 {
            tracker.truncated = true;
            return;
        }
        tracker.links.insert(popup.into(), opener);
    }

    /// Confirmed popup closure counts and the last exact-opener continuation, for agent read-back.
    #[must_use]
    pub fn popup_status(&self, owner: &str) -> serde_json::Value {
        let state = self.inner.lock();
        let Some(tabs) = state.tabs.get(owner) else {
            return serde_json::json!({});
        };
        let mut value = serde_json::json!({"popups":{"closedCount":tabs.popups.closed,"openCount":tabs.popups.links.len(),"complete":true,"trackingTruncated":tabs.popups.truncated}});
        if let Some(continuation) = &tabs.popups.continuation {
            value["popupContinuation"] = continuation.clone();
        }
        value
    }

    /// A lost popup opener requires an explicit choice before an agent can create or select another page.
    #[must_use]
    pub fn popup_needs_selection(&self, owner: &str) -> bool {
        self.inner.lock().tabs.get(owner).is_some_and(|tabs| {
            tabs.agent.is_none()
                && tabs
                    .popups
                    .continuation
                    .as_ref()
                    .is_some_and(|value| value["status"] == "unavailable")
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Return {
    Available(String),
    Unavailable,
}

/// Called only after native teardown succeeded and the exact page was removed from the complete registry.
pub(crate) fn closed(state: &mut crate::state::State, owner: &str, popup: &str) -> Option<Return> {
    let tabs = state.tabs.get_mut(owner)?;
    let opener = tabs.popups.links.remove(popup)?;
    tabs.popups.closed = tabs.popups.closed.saturating_add(1);
    if tabs.agent.as_deref() != Some(popup) {
        return None;
    }
    let live = (opener.allowed)()
        && crate::tabs::review_fingerprint(state, owner, &opener.page).as_deref()
            == Ok(&opener.fingerprint)
        && state
            .page_owners
            .get(&opener.page)
            .is_some_and(|id| id == owner);
    let continuation = if live {
        serde_json::json!({"status":"returned","closedTabId":popup,"openerTabId":opener.page,"message":"The selected popup closed and its exact opener is unchanged and still belongs to this chat. Read the opener before continuing; no input was repeated."})
    } else {
        serde_json::json!({"status":"unavailable","closedTabId":popup,"message":"The popup's original opener changed, closed or lost permission. List the current tabs and explicitly choose a page before continuing; do not repeat the popup input."})
    };
    state.tabs.get_mut(owner)?.popups.continuation = Some(continuation);
    Some(if live {
        Return::Available(opener.page)
    } else {
        Return::Unavailable
    })
}

#[cfg(test)]
#[path = "popup_tests.rs"]
mod tests;
