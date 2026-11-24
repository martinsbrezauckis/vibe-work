//! Steps of one call around its waits: waiting for the browser, the frame before an execute, and confirming an open's answer.
use super::{Arc, Browser, Shared, Value, owner, recheck};

pub(super) async fn wait_for_browser(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    args: &Value,
    browser: Browser,
    consent: Option<&super::super::read_consent::Consent>,
) -> Result<Option<Value>, String> {
    let acts = tool != "chrome_status"
        && !(tool == "chrome_session"
            && matches!(args["action"].as_str(), Some("get" | "cancel" | "stop")));
    if acts {
        if browser == Browser::Firefox && tool == "chrome_session" && args["action"] == "open" {
            // Stop can end the owned child before the pairing poll refreshes its cache.
            let fresh = shared
                .chrome_control
                .owner
                .call("chrome_status", serde_json::json!({}))
                .await;
            let current = fresh.value()["structuredContent"]["firefoxWindow"]
                .as_str()
                .and_then(super::super::PairingState::firefox_window);
            let launch = current.map_or_else(
                || !browser.paired(shared),
                |state| state != super::super::PairingState::WindowOpen,
            );
            if launch {
                super::super::firefox_commands::window(shared, true)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        if let Some(reply) = super::super::browser_start::wait(shared, session, browser).await? {
            return Ok(Some(reply));
        }
        if let Some(retry) = super::super::reconnect::wait(shared, session, browser).await? {
            return Ok(Some(retry));
        }
        // Waiting for a native port grants no authority: mode and the person's Stop are checked again.
        recheck(shared, session, browser, tool, args, consent)?;
    }
    Ok(None)
}

pub(super) async fn prepare_frame(
    shared: &Arc<Shared>,
    session: &str,
    tool: &str,
    browser: Browser,
    args: &Value,
    consent: Option<&super::super::read_consent::Consent>,
) -> Result<(), String> {
    if tool == "chrome_execute"
        && let Some(id) = args["sessionId"].as_str()
    {
        super::super::frames::before_execute(shared, session, id).await?;
        recheck(shared, session, browser, tool, args, consent)?;
    }
    Ok(())
}

pub(super) fn confirm_open_answer(
    shared: &Shared,
    session: &str,
    request: &Value,
    answer: &owner::Answer,
) -> Result<(), String> {
    let body = &answer.value()["structuredContent"];
    let requested = request["profileId"].as_str();
    // An error is not a successful lease receipt. It can nevertheless identify a start
    // still in flight: fence that only as uncertain, never as this chat's selected browser.
    if answer.value()["isError"] == true || !body["error"].is_null() || body["status"] == "failed" {
        let row = body["error"]["details"]["session"]
            .as_object()
            .map(|_| &body["error"]["details"]["session"])
            .or_else(|| {
                body["hostError"]["details"]["session"]
                    .as_object()
                    .map(|_| &body["hostError"]["details"]["session"])
            })
            .or_else(|| body["sessionId"].as_str().map(|_| body));
        if let Some(row) = row
            && !super::super::profile_bindings::open_ended(row)
        {
            super::super::profile_bindings::mark_uncertain(
                &shared.chrome_control,
                session,
                row["sessionId"].as_str(),
                row["profileId"].as_str().or(requested),
            );
        }
    }
    let returned = body["profileId"].as_str();
    let wrong = requested
        .zip(returned)
        .is_some_and(|(requested, returned)| !super::super::profile_ids::same(requested, returned));
    let incomplete = body["status"] != "failed"
        && answer.value()["isError"] != true
        && body["error"].is_null()
        && (body["sessionId"].as_str().is_none() || returned.is_none());
    if body["status"] == "unknown" || wrong || incomplete {
        super::super::profile_bindings::mark_uncertain(
            &shared.chrome_control,
            session,
            body["sessionId"].as_str(),
            returned.or(requested),
        );
    }
    if wrong {
        return Err(vw_vocabulary::texts::BROWSER_PROFILE_WRONG_OPENED.into());
    }
    if incomplete && body["status"] != "unknown" {
        return Err(vw_vocabulary::texts::BROWSER_PROFILE_OPEN_UNCONFIRMED.into());
    }
    Ok(())
}
