//! User and Debug API tab operations. The preview service owns tab order and page identity.
//! Depends on: vw-preview and browser navigation. Called by the browser command handler and agent tools.

use std::sync::Arc;

use serde_json::json;
use vw_preview::{PageTabKind, PageTabOwner, PreviewService};

use crate::browser::{CoreNav, browser};
use crate::browser_view::{BrowserRequest, BrowserResult};
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::previews::{core_error, hub};

fn denied(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::Denied, message.into())
}

fn loaded(shared: &Shared, owner: &str) -> Result<(), CoreError> {
    if shared.store.view(owner).is_some() {
        Ok(())
    } else {
        Err(CoreError::not_loaded(owner))
    }
}

/// Resolves a web tab within the session, excluding the pinned app preview.
pub(crate) fn web_page(service: &PreviewService, owner: &str, key: &str) -> Result<(), CoreError> {
    if service.owner_session_id(key).as_deref() == Some(owner)
        && service.web_tabs(owner).iter().any(|tab| tab == key)
    {
        Ok(())
    } else {
        Err(denied("web tab does not belong to this session"))
    }
}

/// Picks the user's selected page, or an explicitly named page of the same session.
pub(crate) fn user_page(
    service: &PreviewService,
    owner: &str,
    key: Option<&str>,
) -> Result<String, CoreError> {
    let key = key
        .map(str::to_owned)
        .or_else(|| {
            service
                .tabs(owner)
                .into_iter()
                .find(|tab| tab.active)
                .map(|tab| tab.page_key)
        })
        .unwrap_or_else(|| owner.to_owned());
    if key != owner {
        web_page(service, owner, &key)?;
    }
    Ok(key)
}

/// Picks a page for a session credential. A user tab requires explicit handoff.
pub(crate) fn agent_page(
    service: &PreviewService,
    owner: &str,
    tab: Option<&str>,
) -> Result<String, CoreError> {
    let key = match tab {
        Some("new") => service.create_web_tab(owner).map_err(|e| core_error(&e))?,
        Some(key) => key.to_owned(),
        None => service.agent_page(owner).map_or_else(
            || service.create_web_tab(owner).map_err(|e| core_error(&e)),
            Ok,
        )?,
    };
    web_page(service, owner, &key)?;
    service
        .set_agent_page(owner, &key)
        .map_err(|e| core_error(&e))?;
    Ok(key)
}

/// A non-mutating final check for work held across a native read or confirmation.
pub(crate) fn agent_page_allowed(service: &PreviewService, owner: &str, key: &str) -> bool {
    service.owner_session_id(key).as_deref() == Some(owner)
        && service.tabs(owner).into_iter().any(|tab| {
            tab.page_key == key
                && tab.kind == PageTabKind::Web
                && matches!(
                    tab.owner_kind,
                    Some(PageTabOwner::Agent | PageTabOwner::DelegatedToAgent)
                )
        })
}

/// Places a targeted tab in the real Browser area only when that area is currently visible.
/// A hidden panel or modal cannot be brought forward through a stale rectangle.
pub(crate) async fn front_page(
    service: &PreviewService,
    owner: &str,
    key: &str,
) -> Result<(), CoreError> {
    service
        .front_tab_if_visible(owner, key)
        .await
        .map_err(|e| core_error(&e))?;
    Ok(())
}

fn tabs(service: &PreviewService, owner: &str) -> BrowserResult {
    BrowserResult::Tabs {
        tabs: service.tabs(owner),
    }
}

/// Handles the tab family; all requests are user rights, including Debug API requests.
#[allow(clippy::too_many_lines)]
pub(crate) async fn run(
    shared: &Arc<Shared>,
    request: &BrowserRequest,
) -> Option<Result<BrowserResult, CoreError>> {
    let service = match hub(shared) {
        Ok(hub) => &hub.service,
        Err(error) => return Some(Err(error)),
    };
    let (owner, action) = match request {
        BrowserRequest::Tabs { session_id } => (session_id, 0),
        BrowserRequest::OpenTab { session_id, .. } => (session_id, 1),
        BrowserRequest::SwitchTab { session_id, .. } => (session_id, 2),
        BrowserRequest::CloseTab { session_id, .. } => (session_id, 3),
        BrowserRequest::ReorderTabs { session_id, .. } => (session_id, 4),
        BrowserRequest::ReviewHandoff { session_id, .. } => (session_id, 5),
        BrowserRequest::HandoffTab { session_id, .. } => (session_id, 6),
        BrowserRequest::TakeBackTab { session_id, .. } => (session_id, 7),
        _ => return None,
    };
    if let Err(error) = loaded(shared, owner) {
        return Some(Err(error));
    }
    Some(
        async {
            match (action, request) {
                (0, BrowserRequest::Tabs { .. }) => {
                    for tab in service.tabs(owner) {
                        if tab.preview.is_some() {
                            service.refresh_favicon(&tab.page_key).await;
                        }
                    }
                }
                (1, BrowserRequest::OpenTab { url, .. }) => {
                    let key = service
                        .create_user_web_tab(owner)
                        .map_err(|e| core_error(&e))?;
                    service
                        .front_tab_if_visible(owner, &key)
                        .await
                        .map_err(|e| core_error(&e))?;
                    if let Some(url) = url {
                        use vw_browser::Navigate as _;
                        let nav = CoreNav {
                            shared: shared.clone(),
                            strict: false,
                        };
                        if let Err(error) = nav.open(&key, url).await {
                            let _ = service.close_web_tab_and_restore(owner, &key).await;
                            return Err(denied(error));
                        }
                    }
                }
                (2, BrowserRequest::SwitchTab { tab_id, .. }) => {
                    service
                        .front_tab_if_visible(owner, tab_id)
                        .await
                        .map_err(|e| core_error(&e))?;
                }
                (3, BrowserRequest::CloseTab { tab_id, .. }) => {
                    web_page(service, owner, tab_id)?;
                    browser(shared)?.sessions().cancel_actions(tab_id);
                    browser(shared)?.clear_secret_choice(tab_id);
                    service
                        .close_web_tab_and_restore(owner, tab_id)
                        .await
                        .map_err(|e| core_error(&e))?;
                    browser(shared)?.sessions().session_ended(tab_id);
                }
                (4, BrowserRequest::ReorderTabs { tab_ids, .. }) => {
                    service
                        .reorder_web_tabs(owner, tab_ids.clone())
                        .map_err(|e| core_error(&e))?;
                }
                (5, BrowserRequest::ReviewHandoff { tab_id, .. }) => {
                    web_page(service, owner, tab_id)?;
                    let tab = service
                        .tabs(owner)
                        .into_iter()
                        .find(|tab| tab.page_key == *tab_id)
                        .ok_or_else(|| denied("unknown web tab"))?;
                    if tab.kind != PageTabKind::Web || tab.owner_kind != Some(PageTabOwner::User) {
                        return Err(denied(
                            "only a user-controlled web tab can be handed to the agent",
                        ));
                    }
                    let fingerprint = service
                        .handoff_review_fingerprint(owner, tab_id)
                        .map_err(|e| core_error(&e))?;
                    let url = tab.preview.as_ref().and_then(|view| view.url.clone());
                    let title = tab.preview.as_ref().and_then(|view| view.title.clone());
                    return Ok(BrowserResult::HandoffReview {
                        review: json!({
                            "tabId": tab_id, "url": url, "title": title,
                            "site": url.as_deref().and_then(|value| url::Url::parse(value).ok())
                                .and_then(|url| url.host_str().map(str::to_owned)),
                            "ownerKind": "user", "fingerprint": fingerprint,
                            "vaultGranted": false,
                        }),
                    });
                }
                (
                    6,
                    BrowserRequest::HandoffTab {
                        tab_id,
                        fingerprint,
                        confirmed,
                        ..
                    },
                ) => {
                    if !confirmed {
                        return Err(denied("review and confirm this exact tab before handoff"));
                    }
                    web_page(service, owner, tab_id)?;
                    service
                        .delegate_web_tab(owner, tab_id, fingerprint)
                        .map_err(|e| core_error(&e))?;
                }
                (7, BrowserRequest::TakeBackTab { tab_id, .. }) => {
                    web_page(service, owner, tab_id)?;
                    browser(shared)?.sessions().cancel_actions(tab_id);
                    browser(shared)?.sessions().forget_refs(tab_id);
                    let fingerprint = service
                        .handoff_review_fingerprint(owner, tab_id)
                        .map_err(|e| core_error(&e))?;
                    service
                        .take_back_web_tab(owner, tab_id, &fingerprint)
                        .map_err(|e| core_error(&e))?;
                }
                _ => {}
            }
            Ok(tabs(service, owner))
        }
        .await,
    )
}
