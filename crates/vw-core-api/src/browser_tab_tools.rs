//! Session-scoped tab listing and agent close decisions for `browser_act`.
//! Depends on the tab registry, native confirmer, Stop signal and session action log.
//! Called before agent page selection so a list or close never opens or fronts a tab.

use std::sync::Arc;

use serde_json::{Value, json};
use vw_browser::delegation::DelegationAuthority;
use vw_browser::{Actor, Outcome};
use vw_gate::{ConfirmationRequest, Requester, dialog_value};
use vw_preview::{PageTab, PageTabKind, PageTabOwner, PreviewService, PreviewState};

use crate::browser::browser;
use crate::core::Shared;
use crate::previews::hub;

/// Native confirmation method for a user-origin tab. The host shows Close and Keep.
pub(crate) const CLOSE_METHOD: &str = "vibe-work/browserCloseTab";

fn bounded(value: Option<&str>, max: usize) -> Option<String> {
    value.map(|value| value.chars().take(max).collect())
}

fn site(tab: &PageTab) -> Option<String> {
    tab.preview
        .as_ref()
        .and_then(|view| view.url.as_deref())
        .and_then(|url| url::Url::parse(url).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
}

fn tab_json(tab: &PageTab) -> Value {
    json!({
        "id": tab.page_key,
        "site": site(tab),
        "owner": tab.owner_kind,
        "active": tab.active,
        "suspended": tab.preview.as_ref().is_some_and(|view| view.state == PreviewState::Suspended),
    })
}

fn current_tab(service: &PreviewService, owner: &str, key: &str) -> Result<PageTab, String> {
    if key == owner {
        return Err("the pinned App preview cannot be closed".to_owned());
    }
    service
        .tabs(owner)
        .into_iter()
        .find(|tab| tab.page_key == key && tab.kind == PageTabKind::Web)
        .ok_or_else(|| "web tab does not belong to this session".to_owned())
}

fn still_running(shared: &Shared, owner: &str, key: &str) -> Result<(), String> {
    let driver = browser(shared).map_err(|error| error.message)?;
    if driver.sessions().stopped(owner) || driver.sessions().stopped(key) {
        return Err(vw_browser::STOPPED.to_owned());
    }
    if shared.store.view(owner).is_none() {
        return Err("browser session is no longer loaded".to_owned());
    }
    Ok(())
}

pub(crate) async fn prepare_close(
    shared: &Arc<Shared>,
    owner: &str,
    key: &str,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    let service = &hub(shared).map_err(|error| error.message)?.service;
    let target = current_tab(service, owner, key).map_or_else(
        |_| key.to_owned(),
        |tab| {
            format!(
                "{} ({}) [{}]",
                vw_gate::dialog_line(
                    tab.preview
                        .as_ref()
                        .and_then(|v| v.title.as_deref())
                        .unwrap_or("Untitled tab")
                ),
                site(&tab).unwrap_or_else(|| "unknown site".to_owned()),
                key
            )
        },
    );
    let prepared = match key {
        "" => Err("close_tab needs an explicit tab id".to_owned()),
        "new" => Err("close_tab needs an existing web tab id".to_owned()),
        _ => prepare_close_action(shared, owner, key).await,
    };
    let action = match prepared {
        Ok(action) => action,
        Err(error) => {
            log_close(shared, owner, &target, &Err(error.clone()));
            return Err(error);
        }
    };
    let weak = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = weak.upgrade().ok_or("Browser closed.")?;
        let driver = browser(&shared).map_err(|error| error.message)?;
        let _flight = driver.sessions().begin(&owner, Actor::Agent);
        let result = action.await;
        log_close(&shared, &owner, &target, &result);
        result
    }))
}

fn log_close(shared: &Shared, owner: &str, target: &str, result: &Result<Value, String>) {
    let (outcome, detail) = match result {
        Ok(value) => (Outcome::Done, value["result"].as_str().unwrap_or("closed")),
        Err(error) if error == vw_browser::STOPPED || error.contains("canceled") => {
            (Outcome::Stopped, error.as_str())
        }
        Err(error) => (Outcome::Refused, error.as_str()),
    };
    if let Ok(driver) = browser(shared) {
        driver
            .sessions()
            .end(owner, Actor::Agent, "close_tab", target, outcome, detail);
    }
}

async fn prepare_close_action(
    shared: &Arc<Shared>,
    owner: &str,
    key: &str,
) -> Result<crate::ask::tool_calls::receipts::Execution, String> {
    let service = &hub(shared).map_err(|error| error.message)?.service;
    let driver = browser(shared).map_err(|error| error.message)?;
    let _owner_wait = driver.sessions().authorization_wait(owner, Actor::Agent);
    let _page_wait = driver.sessions().authorization_wait(key, Actor::Agent);
    let mut owner_cancel = driver.sessions().cancel_signal(owner);
    let mut page_cancel = driver.sessions().cancel_signal(key);
    owner_cancel.borrow_and_update();
    page_cancel.borrow_and_update();
    still_running(shared, owner, key)?;
    let tab = current_tab(service, owner, key)?;
    let fingerprint = service
        .handoff_review_fingerprint(owner, key)
        .map_err(|error| error.to_string())?;
    let user_origin = tab.owner_kind != Some(PageTabOwner::Agent);
    let permission = crate::browser_delegation::load(shared).close_user_tabs;
    let mut permitted_by_answer = false;
    if user_origin && !permission {
        let title = tab
            .preview
            .as_ref()
            .and_then(|view| view.title.as_deref())
            .unwrap_or("Untitled tab");
        let displayed_site = site(&tab).unwrap_or_else(|| "unknown site".to_owned());
        let request = ConfirmationRequest {
            method: CLOSE_METHOD.to_owned(),
            summary: format!(
                "Close this browser tab? Title: {}. Site: {}. Choose Close to remove it or Keep to leave it open.",
                dialog_value(title, 80),
                dialog_value(&displayed_site, 100),
            ),
            params: json!({"sessionId": owner, "tabId": key, "fingerprint": fingerprint}),
            requester: Requester::Agent,
            always: None,
        };
        let approved = tokio::select! {
            biased;
            _ = owner_cancel.changed() => return Err("browser tab close was canceled".to_owned()),
            _ = page_cancel.changed() => return Err("browser tab close was canceled".to_owned()),
            answer = crate::ask::guards::answer(shared, request, "closing this tab") => answer?.0,
        };
        if !approved.approved() {
            return Err(approved.agent_text("closing this tab"));
        }
        permitted_by_answer = true;
    }
    let weak = Arc::downgrade(shared);
    let owner = owner.to_owned();
    let key = key.to_owned();
    Ok(Box::pin(async move {
        let shared = weak.upgrade().ok_or("Browser closed.")?;
        let shared = &shared;
        let owner = owner.as_str();
        let key = key.as_str();
        let service = &hub(shared).map_err(|e| e.message)?.service;
        let driver = browser(shared).map_err(|e| e.message)?;
        if owner_cancel.has_changed().unwrap_or(true) || page_cancel.has_changed().unwrap_or(true) {
            return Err("browser tab close was canceled".to_owned());
        }
        still_running(shared, owner, key)?;
        let now = current_tab(service, owner, key)?;
        let actual = service
            .handoff_review_fingerprint(owner, key)
            .map_err(|error| error.to_string())?;
        if fingerprint != actual || now.owner_kind != tab.owner_kind {
            return Err("browser tab changed while closing; ask again".to_owned());
        }
        if user_origin
            && !permitted_by_answer
            && !crate::browser_delegation::load(shared).close_user_tabs
        {
            return Err("permission to close user tabs changed; ask again".to_owned());
        }
        let admit = || {
            let allowed = !owner_cancel.has_changed().unwrap_or(true)
                && !page_cancel.has_changed().unwrap_or(true)
                && still_running(shared, owner, key).is_ok()
                && (!user_origin
                    || permitted_by_answer
                    || crate::browser_delegation::load(shared).close_user_tabs);
            if allowed {
                driver.sessions().cancel_actions(key);
                driver.clear_secret_choice(key);
            }
            allowed
        };
        let restore_error = service
            .close_web_tab_and_restore_checked(owner, key, &fingerprint, &admit)
            .await
            .map_err(|error| error.to_string())?;
        driver.sessions().session_ended(key);
        Ok(close_receipt(
            driver,
            owner,
            &tab,
            permitted_by_answer,
            restore_error.map(|error| error.to_string()),
        ))
    }))
}

fn close_receipt(
    driver: &vw_browser::Browser,
    owner: &str,
    tab: &PageTab,
    permitted_by_answer: bool,
    restore_error: Option<String>,
) -> Value {
    let user_origin = tab.owner_kind != Some(PageTabOwner::Agent);
    let result = if permitted_by_answer {
        "closed with your permission"
    } else if user_origin {
        "closed by your permission"
    } else {
        "closed"
    };
    let mut receipt = json!({"action":"close_tab", "ok":true, "tab":{"id":tab.page_key,"site":site(tab)}, "result":result});
    if user_origin {
        let authority = if permitted_by_answer {
            DelegationAuthority::ConfirmedByYou
        } else {
            DelegationAuthority::CloseUserTabs
        };
        if let Err(error) =
            driver.record_user_tab_close(owner, &site(tab).unwrap_or_default(), authority)
        {
            receipt["warning"] = json!(format!(
                "tab closed, but its permission receipt could not be saved: {error}; do not repeat the close"
            ));
        }
    }
    if let Some(error) = restore_error {
        let warning = format!("tab closed, but replacement page could not be shown: {error}");
        receipt["warning"] = json!(match receipt["warning"].as_str() {
            Some(previous) => format!("{previous}; {warning}"),
            None => warning,
        });
    }
    receipt
}

/// Executes a tab-management action using only the owning session's registry.
pub(crate) async fn call(
    shared: &Arc<Shared>,
    owner: &str,
    args: &Value,
    tab: Option<&str>,
) -> Result<Value, String> {
    if shared.store.view(owner).is_none() {
        return Err("browser session is no longer loaded".to_owned());
    }
    let service = &hub(shared).map_err(|error| error.message)?.service;
    match args.get("action").and_then(Value::as_str) {
        Some("list_tabs") => {
            still_running(shared, owner, owner)?;
            let pages: Vec<_> = service
                .tabs(owner)
                .into_iter()
                .filter(|tab| tab.kind == PageTabKind::Web)
                .collect();
            let tabs: Vec<_> = pages.iter().map(tab_json).collect();
            let content: Vec<_> = pages.iter().map(|tab| json!({
                "id": tab.page_key,
                "title": bounded(tab.preview.as_ref().and_then(|view| view.title.as_deref()), 120),
                "url": bounded(tab.preview.as_ref().and_then(|view| view.url.as_deref()), 2_000),
            })).collect();
            Ok(json!({"action":"list_tabs", "ok":true, "tabs":tabs,
                "untrustedPageContent":{"tabs":content}}))
        }
        Some("close_tab") => prepare_close(shared, owner, tab.unwrap_or("")).await?.await,
        _ => Err("unknown browser tab action".to_owned()),
    }
}
