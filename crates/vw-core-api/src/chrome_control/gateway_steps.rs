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
