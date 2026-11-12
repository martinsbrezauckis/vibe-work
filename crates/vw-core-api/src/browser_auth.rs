//! HTTP Basic authentication for the built-in browser through the existing vault/person consent route.
//! Holds an exact native challenge and mount across consent; no credential enters tool arguments or receipts.
//! Depends on: the preview dialog challenge and vault fill path.
//! Called by: browser tool preparation.
use crate::{
    ask::tool_calls::receipts::Execution,
    browser_fill::{self, LoginWriter},
    core::Shared,
};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use vw_browser::{Actor, FillTarget, LoginRole};
use vw_gate::Requester;
use vw_preview::MountId;
use zeroize::Zeroizing;

struct Writer {
    shared: Weak<Shared>,
    owner: String,
    page: String,
    mount: MountId,
    dialog: String,
    origin: String,
    by: Actor,
}
impl Writer {
    fn current(&self, shared: &Shared) -> Result<(), String> {
        let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
        if shared.store.view(&self.owner).is_none()
            || hub.service.page_mount(&self.page).as_ref() != Some(&self.mount)
            || self.by == Actor::Agent
                && (hub.browser.sessions().stopped(&self.page)
                    || hub.browser.sessions().stopped(&self.owner)
                    || !crate::browser_tabs::agent_page_allowed(
                        &hub.service,
                        &self.owner,
                        &self.page,
                    ))
        {
            return Err("Not signed in: the browser page is closed, stopped or no longer belongs to this agent.".into());
        }
        let challenge = hub.service.pending_dialog(&self.page).ok_or(
            "Not signed in: the HTTP challenge closed. Navigate again and read page.dialogs.",
        )?;
        if challenge["dialogId"] != self.dialog
            || challenge["type"] != "auth"
            || challenge["origin"] != self.origin
        {
            return Err("Not signed in: the HTTP challenge changed. Read page.dialogs before choosing a login.".into());
        }
        Ok(())
    }
}
impl LoginWriter for Writer {
    async fn write(
        &self,
        _target: &FillTarget,
        login: &vw_keys::Login,
        value: &str,
    ) -> Result<Value, String> {
        let shared = self.shared.upgrade().ok_or("Vibe Work closed.")?;
        self.current(&shared)?;
        crate::agent_access::check_for(&shared, browser_fill::METHOD, &self.owner)?;
        let hub = crate::previews::hub(&shared).map_err(|e| e.message)?;
        let loads = hub.service.load_state(&self.page).map_or(0, |v| v.loads);
        hub.service.answer_dialog(&self.mount, &self.dialog, vw_preview::DialogAnswer {
            accept: true, text: None, credentials: Some((Zeroizing::new(login.username.clone()), Zeroizing::new(value.to_owned())))
        }).await.map_err(|_| "HTTP sign-in could not answer the native challenge. Read the page before trying again.")?;
        let end = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < end {
            if hub.service.page_mount(&self.page).as_ref() != Some(&self.mount)
                || hub.browser.sessions().stopped(&self.page)
            {
                return Err("HTTP credentials were sent, but the page closed or stopped before sign-in could be checked. Read the page before trying again.".into());
            }
            if hub.service.pending_dialog(&self.page).is_some() {
                return Err("The site asked for authentication again. Check the chosen vault login with the user before trying again.".into());
            }
            if let Some(state) = hub.service.load_state(&self.page)
                && !state.loading
                && state.loads > loads
            {
                if state
                    .url
                    .and_then(|u| url::Url::parse(&u).ok())
                    .is_none_or(|u| u.origin().ascii_serialization() != self.origin)
                {
                    return Err("HTTP credentials were sent, but the page left the checked origin. Read the page before continuing.".into());
                }
                hub.service.block_shots_on_this_document(&self.page);
                return Ok(
                    json!({"Outcome":"Answered the HTTP sign-in challenge and loaded the checked site.","action":"auth.basic","filled":true,"origin":self.origin}),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err("HTTP credentials were sent, but the page did not finish loading in time. Read the page before trying again.".into())
    }
}

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    page: &str,
    args: &Value,
    by: Actor,
    requester: Requester,
) -> Result<Execution, String> {
    Box::pin(prepare_inner(shared, page, args, by, requester)).await
}

async fn prepare_inner(
    shared: &Arc<Shared>,
    page: &str,
    args: &Value,
    by: Actor,
    requester: Requester,
) -> Result<Execution, String> {
    let hub = crate::previews::hub(shared).map_err(|e| e.message)?;
    let wanted = args["login"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 256)
        .ok_or("auth.basic needs login: a saved vault login name or id.")?;
    let url = args["url"]
        .as_str()
        .ok_or("auth.basic needs the HTTP sign-in URL and a vault login name.")?;
    let url = url::Url::parse(url).map_err(|_| "Invalid HTTP sign-in URL.")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !(url.scheme() == "https"
            || url.scheme() == "http"
                && url
                    .host_str()
                    .is_some_and(vw_vault::login::is_loopback_host))
    {
        return Err(
            "Vault sign-in requires HTTPS or a local fixture, without credentials in the URL."
                .into(),
        );
    }
    let origin = url.origin().ascii_serialization();
    if hub.service.pending_dialog(page).is_none() {
        hub.browser
            .act(
                page,
                vw_browser::Action::Navigate {
                    url: url.to_string(),
                },
                by,
                &crate::browser::CoreNav {
                    shared: shared.clone(),
                    strict: false,
                },
            )
            .await
            .map_err(|e| e.text)?;
    }
    let dialog = hub.service.pending_dialog(page).ok_or(
        "This site did not open an HTTP Basic challenge. Read the page and check the sign-in URL.",
    )?;
    if dialog["type"] != "auth"
        || dialog["scheme"] != "basic"
        || dialog["origin"] != origin
        || args
            .get("dialogId")
            .is_some_and(|id| *id != dialog["dialogId"])
    {
        return Err("The native HTTP challenge does not match the checked sign-in URL. Read page.dialogs before continuing.".into());
    }
    let owner = hub
        .service
        .owner_session_id(page)
        .ok_or("Browser page has no chat owner.")?;
    let writer = Writer {
        shared: Arc::downgrade(shared),
        owner: owner.clone(),
        page: page.into(),
        mount: hub
            .service
            .page_mount(page)
            .ok_or(crate::browser_outcomes::SESSION_UNAVAILABLE)?,
        dialog: dialog["dialogId"]
            .as_str()
            .ok_or("HTTP challenge has no id.")?
            .into(),
        origin: origin.clone(),
        by,
    };
    writer.current(shared)?;
    let target = FillTarget {
        reference: writer.dialog.clone(),
        role: LoginRole::Password,
        origin,
        host: url.host_str().unwrap_or_default().into(),
        label: "HTTP sign-in".into(),
    };
    browser_fill::prepare_checked(shared, &owner, wanted, target, requester, writer).await
}
