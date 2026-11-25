//! Agent receipts for accepted browser navigation that waits for an engine slot.
//! Uses the preview's real waiting state; called after tool policy checks and before creating repeat tabs.

use serde_json::{Value, json};
use vw_preview::{PreviewError, PreviewService};

use crate::core::Shared;

/// The service uses this error after recording an accepted waiting page, as well as at mount boundaries.
pub(super) fn busy(message: &str) -> bool {
    message == PreviewError::BrowserBusy.to_string()
}

/// Finds a same-chat waiter by address, restricted to the explicit tab when provided.
pub(super) fn existing(
    service: &PreviewService,
    owner: &str,
    tab: Option<&str>,
    args: &Value,
) -> Result<Option<String>, String> {
    if args["action"] != "navigate" {
        return Ok(None);
    }
    let vw_browser::Action::Navigate { url } = vw_browser::parse(args)? else {
        return Ok(None);
    };
    if tab.is_some_and(|tab| {
        tab != "new" && !crate::browser_tabs::agent_page_allowed(service, owner, tab)
    }) || (tab.is_none() && service.popup_needs_selection(owner))
    {
        return Ok(None);
    }
    let Ok(url) = url::Url::parse(&crate::browser::with_scheme(&url)) else {
        return Ok(None);
    };
    Ok(service.web_tabs(owner).into_iter().find(|page| {
        tab.is_none_or(|tab| tab == "new" || tab == page.as_str())
            && crate::browser_tabs::agent_page_allowed(service, owner, page)
            && service.view(page).is_some_and(|view| {
                view.waiting_for_browser && view.url.as_deref() == Some(url.as_str())
            })
    }))
}

/// Reads back the page's current admission state, including a slot freed while the tool was answering.
pub(super) fn receipt(shared: &Shared, page: &str) -> Result<Value, String> {
    let service = &crate::previews::hub(shared)
        .map_err(|error| error.message)?
        .service;
    let view = service
        .view(page)
        .ok_or(crate::browser_outcomes::TARGET_UNAVAILABLE)?;
    let (status, text) = if view.waiting_for_browser {
        ("waiting", vw_vocabulary::texts::BROWSER_WAITING_RECEIPT)
    } else {
        (
            "notWaiting",
            vw_vocabulary::texts::BROWSER_WAITING_FINISHED_RECEIPT,
        )
    };
    let text = text.replace("{tab}", page);
    let mut receipt = json!({
        "ok": view.waiting_for_browser || service.page_mount(page).is_some(),
        "pending": view.waiting_for_browser, "status": status,
        "Outcome": text, "text": text, "result": {"action": "navigate"},
        "browserState": view.state, "loading": view.loading,
        "note": vw_browser::observe::UNTRUSTED_NOTE,
    });
    super::browser_tools::attach_tab(shared, page, &mut receipt);
    Ok(receipt)
}
