//! Rejects stale stopped open receipts while helper cleanup catches up. Called by the checked gateway.
//! Uses the current turn/stop generation and never replays a browser action or an unknown outcome.
use crate::core::Shared;
use serde_json::Value;
use std::sync::{Arc, PoisonError};

pub(super) async fn fresh(
    shared: &Arc<Shared>,
    session: &str,
    browser: super::agent_browser::Browser,
    pending: &super::person_stop::PendingCall,
    args: &Value,
    mut answer: super::owner::Answer,
) -> Result<super::owner::Answer, String> {
    for attempt in 0..3 {
        pending.finish(shared, session, browser, &answer)?;
        let body = &answer.value()["structuredContent"];
        if body["status"] == "unknown" {
            return Ok(answer);
        }
        let old = body["sessionId"].as_str().is_some_and(|id| {
            shared
                .chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stopped_before(id, session, pending.turn.as_deref())
        });
        let stopped = body["state"].as_str().is_some_and(|state| {
            state.eq_ignore_ascii_case("stopped") || state.eq_ignore_ascii_case("stopping")
        });
        if !old && !stopped {
            return Ok(answer);
        }
        if !stopped {
            return Err(format!(
                "{} reused an ended session. Check the browser before opening a fresh session.",
                browser.name()
            ));
        }
        if !old || attempt == 2 {
            return Err(format!(
                "Not run: {} returned a stopped session. Open a fresh session before acting.",
                browser.name()
            ));
        }
        super::reconnect::stopped(shared, session, browser)?;
        super::check::authorize(shared, session, "chrome_session", args)?;
        answer = super::login::send(shared, session, "chrome_session", args.clone()).await?;
    }
    unreachable!("the final stopped open returns a refusal")
}

/// Bind the chat and folder only after title lookup has finished and current authority is checked again.
pub(super) async fn arguments(
    shared: &Arc<Shared>,
    session: &str,
    browser: super::agent_browser::Browser,
    args: &mut Value,
    consent: Option<&super::read_consent::Consent>,
) -> Result<(), String> {
    super::session_title::apply(shared, session, browser, args).await?;
    super::profile_agent::check(shared, session, "chrome_session", args)?;
    if let Some(profile) = args["profileId"].as_str() {
        super::profile_bindings::remember(&shared.chrome_control, session, profile, None)?;
    }
    super::reconnect::stopped(shared, session, browser)?;
    super::check::authorize_read(shared, session, "chrome_session", args, consent)?;
    args["_clientSessionId"] = serde_json::json!(session);
    if let Ok(folders) = crate::agent_tools::session::folders(shared, session)
        && let Some(folder) = folders.first()
    {
        args["_sessionFolder"] = serde_json::json!(folder);
    }
    Ok(())
}
