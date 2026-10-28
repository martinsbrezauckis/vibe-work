//! Session page tabs and their ownership. The native preview service calls this model; core owns user commands.
//! ported from ShellX's browser

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::policy::Destination;
use crate::service::{OpenRequest, PreviewError, PreviewService};
use crate::state::{BrowserMountState, PreviewView};

fn review_fingerprint(
    state: &crate::state::State,
    owner: &str,
    page_key: &str,
) -> Result<String, PreviewError> {
    let tabs = state
        .tabs
        .get(owner)
        .ok_or_else(|| PreviewError::InvalidTab("unknown session tabs".to_owned()))?;
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

struct ClosingPage<'a> {
    service: &'a PreviewService,
    key: &'a str,
}

impl Drop for ClosingPage<'_> {
    fn drop(&mut self) {
        self.service.inner.lock().closing_pages.remove(self.key);
    }
}

/// A page in the Browser panel. The app page is always first and cannot close.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PageTab {
    /// Opaque key passed to preview operations. The app page key is the session id.
    pub page_key: String,
    /// The pinned app page or a regular browser page.
    pub kind: PageTabKind,
    /// Who controls this web page; the pinned app preview has no handoff owner.
    pub owner_kind: Option<PageTabOwner>,
    /// The page selected in the Browser panel.
    pub active: bool,
    /// The web page selected for agent browser operations.
    pub agent_current: bool,
    /// Latest native page state, when it has opened an address.
    pub preview: Option<PreviewView>,
    /// Last visible PNG, retained across background engine suspension.
    pub snapshot: Option<String>,
    /// Native page icon, when obtained through the admitted page context.
    pub favicon_url: Option<String>,
}

/// The two page roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PageTabKind {
    /// Pinned local app preview.
    AppPreview,
    /// Agent or user web page.
    Web,
}

/// A user page requires an explicit, current-page handoff before the agent can use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PageTabOwner {
    /// Opened and controlled by the user.
    User,
    /// Opened by the agent.
    Agent,
    /// A user page explicitly handed to the agent.
    DelegatedToAgent,
}

impl PreviewService {
    /// Checks page identity and native App restrictions while `open` holds the page operation lock.
    pub(crate) fn preflight_page_open(
        &self,
        request: &OpenRequest,
    ) -> Result<Destination, PreviewError> {
        let session = &request.session_id;
        if session.starts_with("web-page-") && !self.inner.lock().page_owners.contains_key(session)
        {
            return Err(PreviewError::InvalidTab(
                "web page was closed before navigation".to_owned(),
            ));
        }
        let destination = self.inner.policy.classify(&request.url)?;
        let pinned_app = {
            let mut state = self.inner.lock();
            let pinned =
                !state.page_owners.contains_key(session) && state.tabs.contains_key(session);
            if !session.starts_with('\0')
                && !state.page_owners.contains_key(session)
                && matches!(destination, Destination::Loopback { .. })
            {
                state.tabs.entry(session.clone()).or_default();
            }
            pinned
        };
        if pinned_app
            && !matches!(
                destination,
                Destination::Blank | Destination::Loopback { .. }
            )
        {
            return Err(PreviewError::InvalidTab(
                "the App preview opens only this session's local servers".to_owned(),
            ));
        }
        // A local-network literal opens only as the user's Lan page (the core refuses the agent).
        if destination == Destination::Lan && !request.lan {
            return Err(crate::policy::UrlRefusal::Lan(
                request
                    .url
                    .host_str()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect(),
            )
            .into());
        }
        let mut state = self.inner.lock();
        let owner = state.owner_of(session);
        if let Some(tabs) = state.tabs.get_mut(&owner) {
            tabs.snapshots.remove(session);
        }
        Ok(destination)
    }

    /// Selects a tab and places it only if this owner's Browser viewport is still visible.
    /// The visibility decision and native layout share `layout_op`, so a queued modal hide wins cleanly.
    ///
    /// # Errors
    /// An unknown tab, invalid placement, or native engine failure.
    pub async fn front_tab_if_visible(
        &self,
        owner: &str,
        page_key: &str,
    ) -> Result<bool, PreviewError> {
        let _layout = self.layout_op.lock().await;
        self.switch_tab(owner, page_key)?;
        let area = {
            let state = self.inner.lock();
            state
                .shown
                .as_ref()
                .filter(|shown| state.owner_of(&shown.session_id) == owner)
                .map(|shown| shown.area)
        };
        let Some(area) = area else { return Ok(false) };
        self.layout_page_locked(page_key.to_owned(), area).await?;
        Ok(true)
    }
    /// Current native Browser area only while one of this session's pages is actually visible.
    #[must_use]
    pub fn visible_browser_area(&self, owner: &str) -> Option<crate::Rect> {
        let state = self.inner.lock();
        let shown = state.shown.as_ref()?;
        (state.owner_of(&shown.session_id) == owner).then_some(shown.area)
    }
    /// Captures the last safe visible pixels before a tab is hidden. A hidden native view is never sampled.
    pub(super) async fn capture_tab_snapshot(&self, page_key: &str) {
        let mount = {
            let state = self.inner.lock();
            if !state.is_shown(page_key)
                || state.browser_overlays.contains_key(page_key)
                || !(state.page_owners.contains_key(page_key) || state.tabs.contains_key(page_key))
                || state
                    .entries
                    .get(page_key)
                    .is_some_and(|entry| entry.shots_blocked_at == Some(*entry.loads.borrow()))
            {
                return;
            }
            state
                .entries
                .get(page_key)
                .and_then(|entry| entry.ready_mount().cloned())
        };
        let Some(mount) = mount else { return };
        let id = self.mount_id(page_key, &mount);
        let Ok(Ok(png)) =
            tokio::time::timeout(Duration::from_secs(2), self.engine.screenshot(&id)).await
        else {
            return;
        };
        let Ok(shot) = crate::shot::shape(&png) else {
            return;
        };
        let owner = self.inner.lock().owner_of(page_key);
        let mut state = self.inner.lock();
        if !state.is_shown(page_key)
            || state
                .entries
                .get(page_key)
                .is_some_and(|entry| entry.shots_blocked_at == Some(*entry.loads.borrow()))
            || state
                .entries
                .get(page_key)
                .and_then(|entry| entry.mount.as_ref())
                .is_none_or(|current| current.label != mount.label)
        {
            return;
        }
        if let Some(tabs) = state.tabs.get_mut(&owner) {
            tabs.snapshots.insert(
                page_key.to_owned(),
                format!("data:image/png;base64,{}", shot.base64()),
            );
        }
    }
    /// Creates a blank, independently mounted web page owned by `owner`.
    ///
    /// # Errors
    /// An invalid owner, random-key failure, or key collision.
    pub fn create_web_tab(&self, owner: &str) -> Result<String, PreviewError> {
        self.create_tab_with_owner(owner, PageTabOwner::Agent)
    }

    /// Creates a user-controlled web page. Agent use requires explicit handoff.
    ///
    /// # Errors
    /// An invalid owner, random-key failure, or key collision.
    pub fn create_user_web_tab(&self, owner: &str) -> Result<String, PreviewError> {
        self.create_tab_with_owner(owner, PageTabOwner::User)
    }

    fn create_tab_with_owner(
        &self,
        owner: &str,
        kind: PageTabOwner,
    ) -> Result<String, PreviewError> {
        if owner.is_empty()
            || owner.starts_with('\0')
            || self.inner.lock().page_owners.contains_key(owner)
        {
            return Err(PreviewError::InvalidTab("invalid owner session".to_owned()));
        }
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            PreviewError::InvalidTab(format!("cannot create page key: {error}"))
        })?;
        let key = format!(
            "web-page-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        );
        let mut state = self.inner.lock();
        if state.entries.contains_key(&key) || state.page_owners.contains_key(&key) {
            return Err(PreviewError::InvalidTab("page key collision".to_owned()));
        }
        state.page_owners.insert(key.clone(), owner.to_owned());
        let tabs = state.tabs.entry(owner.to_owned()).or_default();
        tabs.web.push(key.clone());
        tabs.owners.insert(key.clone(), kind);
        if tabs.agent.is_none() && kind == PageTabOwner::Agent {
            tabs.agent = Some(key.clone());
        }
        drop(state);
        self.inner.changed(owner);
        Ok(key)
    }

    /// The true owner of a page key. Unregistered keys are ordinary app preview session ids.
    #[must_use]
    pub fn owner_session_id(&self, page_key: &str) -> Option<String> {
        if page_key.is_empty() {
            return None;
        }
        let state = self.inner.lock();
        if let Some(owner) = state.page_owners.get(page_key) {
            return Some(owner.clone());
        }
        (!page_key.starts_with("web-page-")).then(|| page_key.to_owned())
    }

    /// Ordered web page keys of one owner.
    #[must_use]
    pub fn web_tabs(&self, owner: &str) -> Vec<String> {
        self.inner
            .lock()
            .tabs
            .get(owner)
            .map_or_else(Vec::new, |tabs| tabs.web.clone())
    }

    /// The selected agent web page, if any.
    #[must_use]
    pub fn agent_page(&self, owner: &str) -> Option<String> {
        self.inner
            .lock()
            .tabs
            .get(owner)
            .and_then(|tabs| tabs.agent.clone())
    }

    /// All Browser pages, with the pinned app page first.
    #[must_use]
    pub fn tabs(&self, owner: &str) -> Vec<PageTab> {
        let mut state = self.inner.lock();
        state.tabs.entry(owner.to_owned()).or_default();
        let tabs = state.tabs.get(owner);
        let selected = tabs
            .and_then(|tabs| tabs.selected.as_deref())
            .unwrap_or(owner);
        let mut keys = vec![(owner.to_owned(), PageTabKind::AppPreview)];
        if let Some(tabs) = tabs {
            keys.extend(tabs.web.iter().cloned().map(|key| (key, PageTabKind::Web)));
        }
        keys.into_iter()
            .map(|(key, kind)| PageTab {
                active: key == selected,
                agent_current: tabs.and_then(|tabs| tabs.agent.as_deref()) == Some(key.as_str()),
                preview: state
                    .entries
                    .get(&key)
                    .map(|entry| entry.view(state.is_shown(&key))),
                snapshot: state
                    .entries
                    .get(&key)
                    .filter(|entry| entry.mount.is_none())
                    .and_then(|_| tabs.and_then(|tabs| tabs.snapshots.get(&key).cloned())),
                owner_kind: tabs.and_then(|tabs| tabs.owners.get(&key).copied()),
                favicon_url: state
                    .entries
                    .get(&key)
                    .and_then(|entry| entry.favicon.as_ref())
                    .and_then(|(_, icon)| icon.clone()),
                page_key: key,
                kind,
            })
            .collect()
    }

    /// Marks a page as selected; layout then places its native view.
    ///
    /// # Errors
    /// The page is not owned by this session.
    pub fn switch_tab(&self, owner: &str, page_key: &str) -> Result<(), PreviewError> {
        let mut state = self.inner.lock();
        if page_key != owner
            && state
                .page_owners
                .get(page_key)
                .is_none_or(|found| found != owner)
        {
            return Err(PreviewError::InvalidTab(
                "page does not belong to this session".to_owned(),
            ));
        }
        state.tabs.entry(owner.to_owned()).or_default().selected = Some(page_key.to_owned());
        if let Some(entry) = state.entries.get_mut(page_key)
            && entry.browser_mount == BrowserMountState::Released
        {
            entry.browser_mount = BrowserMountState::Ready;
        }
        drop(state);
        self.inner.changed(owner);
        Ok(())
    }

    /// Picks the web page targeted by the agent without changing the visible page.
    ///
    /// # Errors
    /// The page is foreign or still user-controlled.
    pub fn set_agent_page(&self, owner: &str, page_key: &str) -> Result<(), PreviewError> {
        let mut state = self.inner.lock();
        if state
            .page_owners
            .get(page_key)
            .is_none_or(|found| found != owner)
        {
            return Err(PreviewError::InvalidTab(
                "web page does not belong to this session".to_owned(),
            ));
        }
        if state
            .tabs
            .get(owner)
            .and_then(|tabs| tabs.owners.get(page_key))
            == Some(&PageTabOwner::User)
        {
            return Err(PreviewError::InvalidTab(
                "user page requires handoff to the agent".to_owned(),
            ));
        }
        state.tabs.entry(owner.to_owned()).or_default().agent = Some(page_key.to_owned());
        drop(state);
        self.inner.changed(owner);
        Ok(())
    }

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
            .ok_or_else(|| PreviewError::InvalidTab("unknown session tabs".to_owned()))?;
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
        let Some(tabs) = state.tabs.get_mut(owner) else {
            return Err(PreviewError::InvalidTab("unknown session tabs".to_owned()));
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
        drop(state);
        self.inner.changed(owner);
        Ok(())
    }

    /// Reorders all web pages; the app page remains pinned first.
    ///
    /// # Errors
    /// The order omits, duplicates, or includes an unknown web page.
    pub fn reorder_web_tabs(&self, owner: &str, order: Vec<String>) -> Result<(), PreviewError> {
        let mut state = self.inner.lock();
        let tabs = state.tabs.entry(owner.to_owned()).or_default();
        if order.len() != tabs.web.len() || order.iter().any(|key| !tabs.web.contains(key)) {
            return Err(PreviewError::InvalidTab(
                "tab order must contain every web page exactly once".to_owned(),
            ));
        }
        let mut unique = order.clone();
        unique.sort();
        unique.dedup();
        if unique.len() != order.len() {
            return Err(PreviewError::InvalidTab(
                "duplicate web page in tab order".to_owned(),
            ));
        }
        tabs.web = order;
        drop(state);
        self.inner.changed(owner);
        Ok(())
    }

    /// Closes a web page and its native view, retaining the shared public browser store.
    ///
    /// # Errors
    /// The page is foreign or its native view cannot close.
    pub async fn close_web_tab(&self, owner: &str, page_key: &str) -> Result<(), PreviewError> {
        self.close_web_tab_expected(owner, page_key, None, None)
            .await
    }

    async fn close_web_tab_expected(
        &self,
        owner: &str,
        page_key: &str,
        expected: Option<&str>,
        admit: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<(), PreviewError> {
        let _activity = self.session_lock(page_key).await;
        {
            let mut state = self.inner.lock();
            if state
                .page_owners
                .get(page_key)
                .is_none_or(|found| found != owner)
            {
                return Err(PreviewError::InvalidTab(
                    "web page does not belong to this session".to_owned(),
                ));
            }
            if let Some(expected) = expected
                && review_fingerprint(&state, owner, page_key)? != expected
            {
                return Err(PreviewError::InvalidTab(
                    "browser tab changed while closing".to_owned(),
                ));
            }
            if admit.is_some_and(|check| !check()) {
                return Err(PreviewError::InvalidTab(
                    "browser tab close was canceled or its permission changed".to_owned(),
                ));
            }
            if !state.closing_pages.insert(page_key.to_owned()) {
                return Err(PreviewError::InvalidTab(
                    "web page is already closing".to_owned(),
                ));
            }
        }
        let _closing = ClosingPage {
            service: self,
            key: page_key,
        };
        self.unmount(page_key).await?;
        {
            let mut state = self.inner.lock();
            state.entries.remove(page_key);
            state.page_owners.remove(page_key);
            if state.is_shown(page_key) {
                state.shown = None;
            }
            if let Some(tabs) = state.tabs.get_mut(owner) {
                let closed_at = tabs.web.iter().position(|key| key == page_key).unwrap_or(0);
                tabs.web.retain(|key| key != page_key);
                tabs.owners.remove(page_key);
                tabs.snapshots.remove(page_key);
                if tabs.selected.as_deref() == Some(page_key) {
                    tabs.selected = Some(
                        tabs.web
                            .get(closed_at)
                            .or_else(|| tabs.web.last())
                            .cloned()
                            .unwrap_or_else(|| owner.to_owned()),
                    );
                }
                if tabs.agent.as_deref() == Some(page_key) {
                    tabs.agent = tabs.web[closed_at.min(tabs.web.len())..]
                        .iter()
                        .chain(tabs.web[..closed_at.min(tabs.web.len())].iter().rev())
                        .find(|key| {
                            tabs.owners
                                .get(*key)
                                .is_some_and(|kind| *kind != PageTabOwner::User)
                        })
                        .cloned();
                }
            }
        }
        self.refresh_rules().await;
        self.inner.changed(owner);
        Ok(())
    }

    /// Closes a selected page and shows its adjacent tab only while this panel remains visible.
    /// The close, visibility check and replacement share one layout ordering with modal hides.
    ///
    /// # Errors
    /// The page cannot close or the replacement cannot be placed.
    pub async fn close_web_tab_and_restore(
        &self,
        owner: &str,
        page_key: &str,
    ) -> Result<(), PreviewError> {
        match self
            .close_web_tab_and_restore_expected(owner, page_key, None, None)
            .await?
        {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Closes only the exact owner, mount, document and URL previously reviewed.
    /// The admission check runs after the page operation lock and before native teardown.
    /// A successful close returns any error from showing the replacement page separately.
    ///
    /// # Errors
    /// The reviewed page or permission changed, the page is foreign, or its native view cannot close.
    pub async fn close_web_tab_and_restore_checked(
        &self,
        owner: &str,
        page_key: &str,
        fingerprint: &str,
        admit: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Option<PreviewError>, PreviewError> {
        self.close_web_tab_and_restore_expected(owner, page_key, Some(fingerprint), Some(admit))
            .await
    }

    async fn close_web_tab_and_restore_expected(
        &self,
        owner: &str,
        page_key: &str,
        expected: Option<&str>,
        admit: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<Option<PreviewError>, PreviewError> {
        let _layout = self.layout_op.lock().await;
        let area = {
            let state = self.inner.lock();
            state
                .shown
                .as_ref()
                .filter(|shown| shown.session_id == page_key)
                .map(|shown| shown.area)
        };
        self.close_web_tab_expected(owner, page_key, expected, admit)
            .await?;
        let restore_error = if let Some(area) = area {
            let selected = self
                .tabs(owner)
                .into_iter()
                .find(|tab| tab.active)
                .map_or_else(|| owner.to_owned(), |tab| tab.page_key);
            self.layout_page_locked(selected, area).await.err()
        } else {
            None
        };
        Ok(restore_error)
    }
}
