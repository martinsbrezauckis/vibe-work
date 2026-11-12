//! Reacquires a retained browser session after the helper acknowledged release at turn end.
//! Called by the gateway after access checks; stop and turn generations fence late acknowledgements.
use super::{agent_browser::Browser, owner, person_stop::PendingCall};
use crate::core::Shared;
use serde_json::{Value, json};
use std::sync::{Arc, PoisonError};

async fn resume(
    shared: &Arc<Shared>,
    chat: &str,
    browser: Browser,
    args: &Value,
    pending: &PendingCall,
) -> Result<Option<owner::Answer>, String> {
    let Some(id) = args["sessionId"].as_str() else {
        return Ok(None);
    };
    let held = matches!(
        shared
            .chrome_control
            .work
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .state_of(id, chat),
        Some("releasing" | "idle")
    );
    if !held {
        return Ok(None);
    }
    let answer = shared
        .chrome_control
        .owner
        .call("chrome_session", json!({"action":"resume","sessionId":id}))
        .await;
    pending.finish(shared, chat, browser, &answer)?;
    let result = answer.value();
    if result["isError"] == true || result["structuredContent"]["error"].is_object() {
        return Ok(Some(answer));
    }
    if result["structuredContent"]["state"] != "active"
        || result["structuredContent"]["sessionId"] != id
    {
        return Ok(Some(owner::own(
            "Not run: the browser did not confirm control of this retained session. Use chrome_session get to check its state before trying another action.",
            "SESSION_NOT_READY",
        )));
    }
    shared
        .chrome_control
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .resumed(id, chat, pending.turn.as_deref());
    Ok(None)
}

/// Resume failures are ordinary, untrusted helper receipts; successful waits are followed by fresh authority checks.
pub(super) async fn before_execute(
    shared: &Arc<Shared>,
    chat: &str,
    browser: Browser,
    args: &Value,
    pending: &PendingCall,
    consent: Option<&super::read_consent::Consent>,
) -> Result<Option<Value>, String> {
    if let Some(answer) = resume(shared, chat, browser, args, pending).await? {
        return Ok(Some(super::output_cache::for_model(
            shared,
            chat,
            args,
            answer,
            "chrome_execute",
            false,
        )));
    }
    super::gateway::recheck(shared, chat, browser, "chrome_execute", args, consent)?;
    super::gateway::validate_session(shared, chat, "chrome_execute", args)?;
    Ok(None)
}
