//! Page ownership hand-off between a chat and its delegated task: the review fingerprint the native
//! hand-off dialog shows, delegating a web page, taking it back, and the shared owner change.
use sha2::{Digest, Sha256};

use crate::service::{PreviewError, PreviewService};

use super::PageTabOwner;

pub(crate) fn review_fingerprint(
    state: &crate::state::State,
    owner: &str,
    page_key: &str,
) -> Result<String, PreviewError> {
    let tabs = state
        .tabs
        .get(owner)
        .ok_or_else(|| PreviewError::InvalidTab("unknown chat tabs".to_owned()))?;
    let kind = tabs
        .owners
        .get(page_key)
        .ok_or_else(|| PreviewError::InvalidTab("unknown web page".to_owned()))?;
    let entry = state.entries.get(page_key);
    let canonical = serde_json::json!([
        "vw-browser-tab-handoff-v1",
        owner,
        page_key,
        kind,
        entry.and_then(|e| e.url.as_ref().map(url::Url::as_str)),
        entry.and_then(|e| e.mount.as_ref().map(|m| m.label.as_str())),
        entry.map(|e| e.console_epoch),
    ]);
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(canonical.to_string().as_bytes())
    ))
}

impl PreviewService {
    /// Fingerprint of the current page and its ownership for native handoff review.
    ///
    /// # Errors
    /// The session or web page is unknown.
    pub fn handoff_review_fingerprint(
        &self,
        owner: &str,
        page_key: &str,
    ) -> Result<String, PreviewError> {
        review_fingerprint(&self.inner.lock(), owner, page_key)
    }

    /// Atomically delegates a user page after core's user confirmation of its exact state.
    ///
    /// # Errors
    /// The page, owner, or reviewed page state changed.
    pub fn delegate_web_tab(
        &self,
        owner: &str,
        page_key: &str,
        expected_fingerprint: &str,
    ) -> Result<(), PreviewError> {
        self.change_owner(
            owner,
            page_key,
            expected_fingerprint,
            PageTabOwner::User,
            PageTabOwner::DelegatedToAgent,
        )
    }

    /// Takes a delegated page back after core's user confirmation of its exact state.
    ///
    /// # Errors
    /// The page, owner, or reviewed page state changed.
    pub fn take_back_web_tab(
        &self,
        owner: &str,
        page_key: &str,
        expected_fingerprint: &str,
    ) -> Result<(), PreviewError> {
        self.change_owner(
            owner,
            page_key,
            expected_fingerprint,
            PageTabOwner::DelegatedToAgent,
            PageTabOwner::User,
        )
    }

    fn change_owner(
        &self,
        owner: &str,
        page_key: &str,
        expected: &str,
        from: PageTabOwner,
        to: PageTabOwner,
    ) -> Result<(), PreviewError> {
        let mut state = self.inner.lock();
        if state.closing_pages.contains(page_key) {
            return Err(PreviewError::InvalidTab("page is closing".to_owned()));
        }
        let tabs = state
            .tabs
            .get(owner)
            .ok_or_else(|| PreviewError::InvalidTab("unknown chat tabs".to_owned()))?;
        if tabs.owners.get(page_key) != Some(&from) {
            return Err(PreviewError::InvalidTab(
                "page ownership changed".to_owned(),
            ));
        }
        let actual = review_fingerprint(&state, owner, page_key)?;
        if actual != expected {
            return Err(PreviewError::InvalidTab(
                "page handoff review is stale".to_owned(),
            ));
        }
        if let Some(id) = state
            .entries
            .get(page_key)
            .and_then(|entry| entry.mount.as_ref())
        {
            self.engine.dialog_ownership(
                &crate::MountId {
                    session_id: page_key.into(),
                    label: id.label.clone(),
                },
                to != PageTabOwner::User,
            )?;
        }
        let Some(tabs) = state.tabs.get_mut(owner) else {
            return Err(PreviewError::InvalidTab("unknown chat tabs".to_owned()));
        };
        tabs.owners.insert(page_key.to_owned(), to);
        if to == PageTabOwner::DelegatedToAgent {
            tabs.agent = Some(page_key.to_owned());
        }
        if to == PageTabOwner::User && tabs.agent.as_deref() == Some(page_key) {
            tabs.agent = tabs
                .web
                .iter()
                .find(|key| {
                    tabs.owners
                        .get(*key)
                        .is_some_and(|kind| *kind != PageTabOwner::User)
                })
                .cloned();
        }
        // A handed page joins the agent's input block at once; a page taken back leaves it at once.
        if let Some(label) = state
            .entries
            .get(page_key)
            .and_then(|entry| entry.mount.as_ref())
            .map(|mount| mount.label.clone())
        {
            crate::service_person_block::scope_view(&state, page_key, &label);
        }
        drop(state);
        self.inner.changed(owner);
        Ok(())
    }
}
