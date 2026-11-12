//! Agent window actions and the person's title-bar close, scoped to one chat's web pages.
//! Reuses checked navigation, Stop, tab-close review and native preview window operations.
//! Depends on: browser ownership, checked navigation and preview windows.
//! Called by: browser command dispatch and title-bar close.
use crate::{ask::tool_calls::receipts::Execution, core::Shared};
use serde_json::{Value, json};
use std::sync::Arc;
use vw_browser::{Actor, Outcome};

pub(crate) fn install(shared: &Arc<Shared>) {
    let Some(hub) = &shared.preview else {
        return;
    };
    let weak = Arc::downgrade(shared);
    let runtime = tokio::runtime::Handle::current();
    hub.service.on_window_close(move |page| {
        let Some(shared) = weak.upgrade() else {
            return;
        };
        runtime.spawn(async move {
            let Some(hub) = &shared.preview else {
                return;
            };
            let Some(owner) = hub.service.owner_session_id(&page) else {
                return;
            };
            if hub.service.page_window_id(&page).is_none() {
                return;
            }
            hub.browser.sessions().stop(&page);
            if let Err(error) = hub.service.close_web_tab_and_restore(&owner, &page).await {
                tracing::warn!(%error, "browser window could not close");
            }
        });
    });
}

fn current(shared: &Shared, owner: &str, page: Option<&str>) -> Result<(), String> {
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    if shared.store.view(owner).is_none()
        || hub.browser.sessions().stopped(owner)
        || hub
            .service
            .web_tabs(owner)
            .iter()
            .any(|key| hub.browser.sessions().stopped(key))
    {
        return Err(vw_browser::STOPPED.into());
    }
    if let Some(page) = page
        && !crate::browser_tabs::agent_page_allowed(&hub.service, owner, page)
    {
        return Err("This browser window is no longer available to this chat. Use a window owned by this chat.".into());
    }
    Ok(())
}

pub(crate) fn prepare(
    shared: &Arc<Shared>,
    owner: &str,
    args: &Value,
) -> Result<Execution, String> {
    current(shared, owner, None)?;
    let action = args["action"].as_str().unwrap_or_default();
    if action == "window.list" {
        return prepare_list(shared, owner, args);
    }
    if args.get("tab").is_some() {
        return Err("Window actions take windowId, not tab. Use the windowId returned by window.open or list_tabs.".into());
    }
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    let page = if action == "window.open" {
        if args.get("windowId").is_some() {
            return Err("window.open creates a new window; omit windowId.".into());
        }
        None
    } else {
        let id = args["windowId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("This action needs windowId from window.open or list_tabs.")?;
        let page = hub.service.window_page(owner, id).ok_or(
            "This window does not belong to this chat. Use its own windowId from list_tabs.",
        )?;
        current(shared, owner, Some(&page))?;
        Some(page)
    };
    if action == "window.close" {
        if page
            .as_deref()
            .and_then(|page| hub.service.page_window_id(page))
            .is_none()
        {
            return Err("This browser tab is in the main window, which window.close cannot close. Use close_tab with its tab id to close only the page.".into());
        }
        let close = crate::browser_tab_tools::prepare_close(
            shared,
            owner,
            page.as_deref().unwrap_or_default(),
        )?;
        let id = args["windowId"].clone();
        return Ok(Box::pin(async move {
            let mut result = close.await?;
            result["action"] = json!("window.close");
            result["windowId"] = id;
            result["closed"] = json!(true);
            result["Outcome"] = json!("Closed this chat's browser window and its page.");
            Ok(result)
        }));
    }
    if !matches!(action, "window.open" | "window.focus" | "window.resize") {
        return Err("Unknown window action.".into());
    }
    let action = action.to_owned();
    let args = args.clone();
    let weak = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = weak
            .upgrade()
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?;
        current(&shared, &owner, page.as_deref())?;
        let hub = crate::previews::hub(&shared).map_err(|e| e.message)?;
        let _flight = hub.browser.sessions().begin(&owner, Actor::Agent);
        let result = if action == "window.open" {
            open(&shared, &owner, &args).await
        } else {
            update(&shared, &owner, &args, page.as_deref()).await
        };
        let (outcome, detail) = match &result {
            Ok(_) => (Outcome::Done, "Browser window updated"),
            Err(error) => (Outcome::Refused, error.as_str()),
        };
        hub.browser.sessions().end(
            &owner,
            Actor::Agent,
            &action,
            "Browser window",
            outcome,
            detail,
        );
        result
    }))
}

fn prepare_list(shared: &Arc<Shared>, owner: &str, args: &Value) -> Result<Execution, String> {
    if ["tab", "windowId", "url"]
        .iter()
        .any(|key| args.get(key).is_some())
    {
        return Err(
            "window.list reads this chat's independent windows. Omit tab, windowId and url.".into(),
        );
    }
    let weak = Arc::downgrade(shared);
    let owner = owner.to_owned();
    Ok(Box::pin(async move {
        let shared = weak
            .upgrade()
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?;
        current(&shared, &owner, None)?;
        let hub = crate::previews::hub(&shared).map_err(|e| e.message)?;
        let windows = hub
            .service
            .inspect_page_windows(&owner)
            .await
            .map_err(|e| e.to_string())?;
        current(&shared, &owner, None)?;
        Ok(
            json!({"action":"window.list","windows":windows,"scope":"chat-owned independent windows"}),
        )
    }))
}

async fn open(shared: &Arc<Shared>, owner: &str, args: &Value) -> Result<Value, String> {
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    let url = args
        .get("url")
        .map(|value| {
            value
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("window.open url must be a nonempty URL.")
        })
        .transpose()?
        .unwrap_or("about:blank");
    let page = hub
        .service
        .create_web_tab(owner)
        .map_err(|e| e.to_string())?;
    let result = async {
        let info = hub
            .service
            .open_page_window(owner, &page)
            .await
            .map_err(|e| e.to_string())?;
        current(shared, owner, Some(&page))?;
        let receipt = if url == "about:blank" {
            // An empty native document has no external destination or inherited local-port authority.
            hub.service
                .open(vw_preview::OpenRequest {
                    session_id: page.clone(),
                    url: url::Url::parse("about:blank").map_err(|e| e.to_string())?,
                    allowed_ports: Vec::new(),
                    downloads_dir: None,
                    wait: false,
                    lan: false,
                })
                .await
                .map_err(|e| e.to_string())?;
            Value::Null
        } else {
            hub.browser
                .act(
                    &page,
                    vw_browser::Action::Navigate { url: url.into() },
                    Actor::Agent,
                    &crate::browser::CoreNav {
                        shared: shared.clone(),
                        strict: true,
                    },
                )
                .await
                .map_err(|e| e.text)?
        };
        current(shared, owner, Some(&page))?;
        hub.service
            .set_agent_page(owner, &page)
            .map_err(|e| e.to_string())?;
        let mut result = serde_json::to_value(info).map_err(|e| e.to_string())?;
        result["action"] = json!("window.open");
        result["tab"] = json!(page);
        result["Outcome"] = json!("Opened a browser window for this chat.");
        if receipt["untrustedPageContent"]["dialog"].is_object() {
            result["untrustedPageContent"] = receipt["untrustedPageContent"].clone();
        }
        Ok(result)
    }
    .await;
    if result.is_err() {
        let _ = hub.service.close_web_tab_and_restore(owner, &page).await;
    }
    result
}

async fn update(
    shared: &Shared,
    owner: &str,
    args: &Value,
    page: Option<&str>,
) -> Result<Value, String> {
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    let id = args["windowId"].as_str().unwrap_or_default();
    let resize = args["action"] == "window.resize";
    let info = if resize {
        let size = |key: &str, min| {
            args[key]
                .as_u64()
                .filter(|n| (min..=4096).contains(n))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    format!("window.resize needs {key} between {min} and 4096 logical pixels.")
                })
        };
        hub.service
            .resize_page_window(owner, id, size("width", 400)?, size("height", 300)?)
            .await
    } else {
        hub.service.focus_page_window(owner, id).await
    }
    .map_err(|e| e.to_string())?;
    let page = page.ok_or("Browser window closed.")?;
    current(shared, owner, Some(page))?;
    hub.service
        .set_agent_page(owner, page)
        .map_err(|e| e.to_string())?;
    let mut result = serde_json::to_value(info).map_err(|e| e.to_string())?;
    result["action"] = args["action"].clone();
    result["tab"] = json!(page);
    result["Outcome"] = json!(if resize {
        "Resized this chat's browser window and measured its content area."
    } else {
        "Focused this chat's browser window."
    });
    Ok(result)
}
