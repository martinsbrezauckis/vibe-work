//! The person's routes into the paused-tool module: the `PausedTools` command (chat line, built-in browser pane,
//! Debug API acting as the person), the person's next message (resumes), and the browsers' frame update after each
//! change. The agent's tools never reach this file.
//! Depends on: `paused_tools`, the store (chat exists, turn running), `chrome_control` frames, the built-in browser's
//! input block. Called by: dispatch.
use std::sync::Arc;

use crate::command::Command;
use crate::core::Shared;
use crate::error::{CoreError, ErrorCode};
use crate::paused_tools::BROWSER;
use crate::{CommandResult, PausedToolsRequest};

/// What the chat line names when the person paused from Vibe Work itself.
const DEFAULT_PLACE: &str = "the browser";

/// Runs one `PausedTools` request.
///
/// # Errors
/// `notFound` when the chat is not open; `invalidParams` when Pause is asked while no turn of the chat is running
/// (there is nothing to pause: the agent is not working).
pub(crate) fn run(
    shared: &Arc<Shared>,
    request: PausedToolsRequest,
) -> Result<CommandResult, CoreError> {
    let chat = match &request {
        PausedToolsRequest::Read { session_id }
        | PausedToolsRequest::Pause { session_id, .. }
        | PausedToolsRequest::Resume { session_id } => shared.channels.canonical(session_id),
    };
    if shared.store.view(&chat).is_none() {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "Vibe Work has no open chat with this id.",
        ));
    }
    match request {
        PausedToolsRequest::Read { .. } => {}
        PausedToolsRequest::Pause { place, .. } => {
            if !turn_running(shared, &chat) {
                return Err(CoreError::new(
                    ErrorCode::InvalidParams,
                    "The agent is not working in this chat, so there is nothing to pause.",
                ));
            }
            let place = place
                .as_deref()
                .map(str::trim)
                .filter(|place| !place.is_empty())
                .unwrap_or(DEFAULT_PLACE);
            if shared.paused_tools.pause(&chat, BROWSER, place) {
                // The turn may have ended between the check above and the pause.
                turn_finished(shared, &chat);
                crate::chrome_control::sync_frames(shared, &chat);
            }
        }
        PausedToolsRequest::Resume { .. } => {
            if shared.paused_tools.resume(&chat, None) {
                crate::chrome_control::sync_frames(shared, &chat);
                crate::browser_input_block::sync(shared, &chat);
            }
        }
    }
    Ok(CommandResult::PausedTools {
        view: shared.paused_tools.view(&chat),
    })
}

/// True while the chat's latest turn runs.
pub(crate) fn turn_running(shared: &Shared, chat: &str) -> bool {
    shared.store.view(chat).is_some_and(|view| {
        view.state["turns"]
            .as_array()
            .and_then(|turns| turns.last())
            .is_some_and(|turn| turn["status"] == "in_progress")
    })
}

/// The chat's turn finished (an event or a snapshot): its pauses end and a waiting call never runs. A late end event
/// of an older turn while a newer one runs changes nothing.
pub(crate) fn turn_finished(shared: &Shared, session: &str) {
    let chat = shared.channels.canonical(session);
    if !turn_running(shared, &chat) {
        shared.paused_tools.turn_ended(&chat);
        // The frame goes off with the turn; so does the built-in browser's input block.
        crate::browser_input_block::sync(shared, &chat);
    }
}

/// The `Call` methods that carry a message the person sends into a chat (the composer's send, steer, queue and "send
/// now" of a queued message). An edit or removal of a queued message is not a new message.
const MESSAGE_METHODS: [&str; 4] = [
    "turn/start",
    "turn/steer",
    "session/turn/enqueue",
    "session/turn/queue/steer",
];

/// The chat a person's message goes to, on the typed commands and on the generic `Call` the composer uses; `None` for
/// every other command. Only the window and the Debug API (acting as the person) reach `Core::execute`; the agent's
/// tool calls never do.
fn message_chat(command: &Command) -> Option<&str> {
    match command {
        Command::StartTurn { session_id, .. } | Command::SteerTurn { session_id, .. } => {
            Some(session_id)
        }
        Command::Call { method, params, .. } if MESSAGE_METHODS.contains(&method.as_str()) => {
            params.get("sessionId").and_then(serde_json::Value::as_str)
        }
        _ => None,
    }
}

/// The person's next message in a chat resumes its paused tools (a waiting call then runs).
pub(crate) fn note_command(shared: &Arc<Shared>, command: &Command) {
    let Some(session) = message_chat(command) else {
        return;
    };
    let chat = shared.channels.canonical(session);
    if shared.paused_tools.resume(&chat, None) {
        crate::chrome_control::sync_frames(shared, &chat);
        crate::browser_input_block::sync(shared, &chat);
    }
}

#[cfg(test)]
#[path = "paused_tools_commands_tests.rs"]
mod tests;
