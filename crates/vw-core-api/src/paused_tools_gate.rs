//! Where agent calls wait while the person paused the browser. The long wait (up to ten minutes) happens in the
//! pre-tool receipt, which has no transport deadline; the browser gateways wait again briefly for a pause that
//! started after the receipt was issued, inside the agent tool's own deadline.
//! Depends on: `paused_tools`, `export_hooks` (what a hook notice is). Called by: the receipt route
//! (`ask::tool_calls::receipts`), the helper browsers' gateway, the built-in browser tools, and every place that shows
//! Vibe's session history (`mark_held_calls`).
use std::time::Duration;

use serde_json::Value;

use crate::core::Shared;
use crate::paused_tools::{BROWSER, HeldCall, Hold, TIMED_OUT};

/// The agent's answer when the person pressed Stop while its browser call waited. The transcript shows the person
/// their own line for it (`pausedBrowserStoppedCall`), so both texts live in the shared vocabulary.
pub(crate) const STOPPED: &str = vw_vocabulary::texts::AGENT_PAUSED_BROWSER_STOPPED;
/// The agent's answer when its turn ended while its browser call waited (person's line: `pausedBrowserTurnEndedCall`).
pub(crate) const TURN_ENDED: &str = vw_vocabulary::texts::AGENT_PAUSED_BROWSER_TURN_ENDED;

/// True for an agent browser call that acts in or reads a page, or opens one: it waits while the browser is paused.
/// The App preview's page tools count as browser calls. Status reads (`chrome_status`, `preview_list`) and ending a
/// session never wait (Stop and status stay available).
pub(crate) fn browser_call(tool: &str, args: &Value) -> bool {
    match tool {
        "chrome_execute"
        | "browser_read"
        | "browser_act"
        | "browser_capture"
        | "browser_capture_secret"
        | "preview_open"
        | "preview_screenshot"
        | "preview_console" => true,
        "chrome_session" => args["action"] == "open",
        _ => false,
    }
}

/// Waits while the chat's browser is paused, at most `deadline`. `Ok` = run the call now (after the route's normal
/// checks); `Err` = the agent's "not run" answer.
pub(crate) async fn wait(shared: &Shared, chat: &str, deadline: Duration) -> Result<(), String> {
    wait_call(shared, chat, deadline, None).await
}

/// [`wait`] for a call whose identity is known, so a Stop or turn end that ends it is recorded for its chat line.
async fn wait_call(
    shared: &Shared,
    chat: &str,
    deadline: Duration,
    call: Option<HeldCall>,
) -> Result<(), String> {
    let chat = shared.channels.canonical(chat);
    match shared
        .paused_tools
        .hold_call(&chat, BROWSER, deadline, call)
        .await
    {
        Hold::NotPaused | Hold::Resumed => Ok(()),
        Hold::Stopped => Err(STOPPED.to_owned()),
        Hold::TurnEnded => Err(TURN_ENDED.to_owned()),
        Hold::TimedOut => Err(TIMED_OUT.to_owned()),
    }
}

/// The pre-tool receipt's wait: a browser call of a chat whose browser the person paused waits here (no transport
/// deadline) until Resume, Stop, turn end or [`crate::paused_tools::DEFAULT_WAIT`]. Then returns the chat's Stop
/// generation the receipt is bound to, read after the wait, so a Stop during the wait is never missed. `call` names the
/// waiting call (the hook's Vibe session and call id) so the chat can mark it when Stop or the turn's end ends it.
pub(crate) async fn receipt(
    shared: &Shared,
    chat: &str,
    tool: &str,
    args: &Value,
    call: Option<(&str, &str)>,
) -> Result<u64, String> {
    if browser_call(tool, args) {
        let call = call.map(|(session, call)| HeldCall {
            session: session.to_owned(),
            call: call.to_owned(),
        });
        wait_call(shared, chat, crate::paused_tools::DEFAULT_WAIT, call).await?;
    }
    Ok(crate::stops::generation(shared, chat))
}

/// The history field that marks Vibe's pre-tool hook notices of a call that waited for the paused browser and ended
/// without running: `"stopped"` (the person's Stop) or `"turnEnded"`. The transcript and the Markdown export show the
/// person's own line for that call from it (`pausedBrowserStoppedCall` / `pausedBrowserTurnEndedCall`), whether the
/// hook's answer reached the runtime, reached it late, or the runtime cancelled the hook first.
pub(crate) const HELD_CALL_FIELD: &str = "vwHeldCall";

/// Marks, anywhere in `value` (a history entry, a notification, a snapshot or a history page), each pre-tool hook
/// notice of a held call that Stop or the turn's end ended ([`HELD_CALL_FIELD`]). Every other entry stays as is.
pub(crate) fn mark_held_calls(shared: &Shared, value: &mut Value) {
    if shared.paused_tools.has_held_ends() {
        mark(&shared.paused_tools, value);
    }
}

fn mark(tools: &crate::paused_tools::PausedTools, value: &mut Value) {
    if crate::export_hooks::is_hook_notice(value)
        && value["detail"]["scope"] == "pre_tool"
        && let (Some(session), Some(call)) = (
            value["sessionId"].as_str(),
            value["detail"]["toolCallId"].as_str(),
        )
        && let Some(end) = tools.held_end(session, call)
    {
        value[HELD_CALL_FIELD] = Value::String(end.mark().to_owned());
    }
    match value {
        Value::Object(map) => map.values_mut().for_each(|v| mark(tools, v)),
        Value::Array(list) => list.iter_mut().for_each(|v| mark(tools, v)),
        _ => {}
    }
}

/// Puts the "page may have changed" note first in an MCP result's text, once after a Resume.
pub(crate) fn note_resumed(shared: &Shared, chat: &str, result: &mut Value) {
    note_resumed_as(shared, chat, result, None);
}

/// [`note_resumed`] for a helper browser (Your Chrome, Edge, Firefox): the note also says the agent's network rules
/// ended at the Pause ([`crate::paused_tools::CHROME_RESUMED_NOTE`]).
pub(crate) fn note_resumed_helper(shared: &Shared, chat: &str, result: &mut Value) {
    note_resumed_as(
        shared,
        chat,
        result,
        Some(crate::paused_tools::CHROME_RESUMED_NOTE),
    );
}

fn note_resumed_as(shared: &Shared, chat: &str, result: &mut Value, text: Option<&'static str>) {
    let chat = shared.channels.canonical(chat);
    let Some(note) = shared
        .paused_tools
        .take_resumed_note(&chat, BROWSER)
        .map(|plain| text.unwrap_or(plain))
    else {
        return;
    };
    if let Some(content) = result["content"].as_array_mut() {
        content.insert(0, serde_json::json!({"type":"text","text":note}));
    } else if let Some(object) = result.as_object_mut() {
        object.insert("pausedNote".to_owned(), Value::String(note.to_owned()));
    }
}

#[cfg(test)]
#[path = "paused_tools_gate_tests.rs"]
mod tests;
