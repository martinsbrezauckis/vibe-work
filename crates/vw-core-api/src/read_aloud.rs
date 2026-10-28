//! One-time reply reading: session-owned final text, key/window checks, then a window queue event.
//! Depends on the session store and key readiness; called by agent tools and the speech command.
//! Never writes settings or waits for audio; tools share one reading per real user turn in their session.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::core::Shared;
use crate::event::CoreEvent;

const LIMIT: usize = 10_000;

enum Receipt {
    Available,
    Unavailable,
    Busy,
}

/// Window receipts for speech preparation and reserved or spent agent turns for this core.
#[derive(Default)]
pub(crate) struct Pending {
    next: AtomicU64,
    replies: Mutex<HashMap<String, oneshot::Sender<Receipt>>>,
    turns: Mutex<HashSet<(String, String)>>,
}

/// Holds a turn's place across speech preparation; only a successful receipt spends it.
struct Reservation<'a> {
    turns: &'a Mutex<HashSet<(String, String)>>,
    key: (String, String),
    accepted: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.turns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&self.key);
        }
    }
}

impl Pending {
    fn reserve(&self, session: &str, turn: &str) -> Option<Reservation<'_>> {
        let key = (session.to_owned(), turn.to_owned());
        self.turns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone())
            .then(|| Reservation {
                turns: &self.turns,
                key,
                accepted: false,
            })
    }

    fn begin(&self) -> (String, oneshot::Receiver<Receipt>) {
        let id = format!("reading-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone(), sender);
        (id, receiver)
    }

    fn take(&self, id: &str) -> Option<oneshot::Sender<Receipt>> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
    }
}

pub(crate) fn ready(shared: &Shared, id: &str, available: bool, busy: bool) -> Value {
    let receipt = if available {
        Receipt::Available
    } else if busy {
        Receipt::Busy
    } else {
        Receipt::Unavailable
    };
    if shared
        .readings
        .take(id)
        .is_some_and(|reply| reply.send(receipt).is_ok())
    {
        outcome("accepted", "The reading request was received.")
    } else {
        outcome("expired", "The reading request is no longer waiting.")
    }
}

/// Reading makes only stoppable sound; Vibe's Ask profile needs no extra approval card for it.
pub(crate) fn approve_without_dialog(callback: &Value) -> bool {
    callback.pointer("/detail/kind").and_then(Value::as_str) == Some("approval")
        && callback
            .pointer("/detail/effect/toolName")
            .and_then(Value::as_str)
            == Some("mcp_vw.read_aloud")
}

pub(crate) async fn tool(shared: &Shared, session: &str, args: &Value) -> Result<Value, String> {
    let args = args
        .as_object()
        .ok_or("Reading aloud needs an object with optional text.")?;
    if args.keys().any(|key| key != "text") {
        return Err(
            "Reading aloud can only read this session's reply or the given text.".to_owned(),
        );
    }
    let text = match args.get("text") {
        None => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => return Err("Reading aloud needs text in plain words.".to_owned()),
    };
    Ok(start(shared, session, text, true).await)
}

fn outcome(status: &str, message: &str) -> Value {
    json!({"status": status, "message": message})
}

pub(crate) async fn start(
    shared: &Shared,
    session: &str,
    text: Option<&str>,
    before_request: bool,
) -> Value {
    let Some(view) = shared.store.view(session) else {
        return outcome(
            "noReply",
            "This session has no finished reply to read aloud.",
        );
    };
    let mut reservation = if before_request {
        let Some(turn) = requesting_turn(&view.state) else {
            return outcome(
                "noReply",
                "This session has no finished reply to read aloud.",
            );
        };
        let Some(reservation) = shared.readings.reserve(session, turn) else {
            return outcome(
                "alreadyRead",
                "This reply was already read aloud; ask again in a new message to hear it again.",
            );
        };
        Some(reservation)
    } else {
        None
    };
    if !shared
        .config
        .read_aloud_available
        .as_ref()
        .is_some_and(|available| available())
    {
        return outcome(
            "nobodyListening",
            "Reading aloud needs an open Vibe Work window.",
        );
    }
    if shared
        .config
        .keys
        .as_ref()
        .is_none_or(|keys| !keys.speech_available())
    {
        return outcome(
            "keyMissing",
            "Reading aloud needs your Mistral key; set it in Settings > Account.",
        );
    }
    let resolved = text
        .map(str::to_owned)
        .or_else(|| previous_reply(&view.state, before_request));
    let Some(text) = resolved.filter(|text| !text.trim().is_empty()) else {
        return outcome(
            "noReply",
            "This session has no finished reply to read aloud.",
        );
    };
    let truncated = text.chars().count() > LIMIT;
    let title = view.state["session"]["title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .unwrap_or("Untitled chat")
        .to_owned();
    let (request_id, reply) = shared.readings.begin();
    shared.emit(CoreEvent::ReadAloud {
        request_id: request_id.clone(),
        session_id: session.to_owned(),
        title,
        text: text.chars().take(LIMIT).collect(),
        truncated,
    });
    match tokio::time::timeout(Duration::from_secs(40), reply).await {
        Ok(Ok(Receipt::Available)) => {
            if let Some(reservation) = reservation.as_mut() {
                reservation.accepted = true;
            }
            outcome("started", "Reading aloud has started in Vibe Work.")
        }
        Ok(Ok(Receipt::Busy)) => outcome("voiceUnavailable", "Another reading is still playing."),
        Ok(Ok(Receipt::Unavailable)) => outcome(
            "voiceUnavailable",
            "Reading aloud is unavailable; try again in a minute.",
        ),
        _ => {
            shared.readings.take(&request_id);
            outcome(
                "nobodyListening",
                "Reading aloud needs an open Vibe Work window.",
            )
        }
    }
}

fn requesting_turn(state: &Value) -> Option<&str> {
    state["history"].as_array()?.iter().rev().find(|entry| {
        entry["type"] == "message" && entry["role"] == "user" && entry["source"] != "harness"
    })?["turnId"]
        .as_str()
        .filter(|id| !id.is_empty())
}

/// Last completed assistant message of the last completed turn before the requesting user's turn.
fn previous_reply(state: &Value, before_request: bool) -> Option<String> {
    let history = state["history"].as_array()?;
    let turns = state["turns"].as_array()?;
    let requesting = before_request.then(|| requesting_turn(state)).flatten();
    let end = requesting
        .and_then(|id| turns.iter().position(|turn| turn["id"] == id))
        .unwrap_or(turns.len());
    let turn = turns[..end]
        .iter()
        .rev()
        .find(|turn| turn["status"] == "completed")?;
    let entry = history.iter().rev().find(|entry| {
        entry["turnId"] == turn["id"]
            && entry["type"] == "message"
            && entry["role"] == "assistant"
            && entry["generationStatus"] == "completed"
    })?;
    Some(
        entry["content"]
            .as_array()?
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_messages_cannot_renew_a_spent_turn_and_sessions_have_separate_budgets() {
        let pending = Pending::default();
        let mut state = json!({"history":[
            {"type":"message", "role":"user", "turnId":"one"},
            {"type":"message", "role":"user", "turnId":"automatic", "source":"harness"}
        ]});
        let turn = requesting_turn(&state).unwrap();
        assert_eq!(turn, "one");
        let mut reading = pending.reserve("session", turn).unwrap();
        assert!(
            pending.reserve("session", turn).is_none(),
            "preparation reserves the turn"
        );
        reading.accepted = true;
        drop(reading);
        assert!(
            pending.reserve("session", turn).is_none(),
            "a successful reading spends the turn"
        );
        assert!(pending.reserve("another-session", turn).is_some());
        state["history"].as_array_mut().unwrap().push(json!({
            "type":"message", "role":"user", "turnId":"two"
        }));
        assert!(
            pending
                .reserve("session", requesting_turn(&state).unwrap())
                .is_some()
        );
        let harness_only = json!({"history":[
            {"type":"message", "role":"user", "turnId":"automatic", "source":"harness"}
        ]});
        assert_eq!(requesting_turn(&harness_only), None);
    }

    #[test]
    fn only_the_reading_tool_may_skip_its_approval_card() {
        let mut callback =
            json!({"detail":{"kind":"approval", "effect":{"toolName":"mcp_vw.read_aloud"}}});
        assert!(approve_without_dialog(&callback));
        callback["detail"]["effect"]["toolName"] = json!("mcp_vw.process_start");
        assert!(!approve_without_dialog(&callback));
        callback["detail"]["effect"]["toolName"] = json!("mcp_vw.read_aloud_extra");
        assert!(!approve_without_dialog(&callback));
        callback["detail"]["effect"]["toolName"] = json!("mcp_vw.read_aloud");
        callback["detail"]["kind"] = json!("user_input");
        assert!(!approve_without_dialog(&callback));
    }

    #[test]
    fn resolves_final_reply_before_request_never_acknowledgement_thinking_or_tools() {
        let state = json!({"turns": [{"id":"one", "status":"completed"}, {"id":"two", "status":"completed"}], "history": [
            {"type":"message", "role":"user", "turnId":"one", "content":[]},
            {"type":"message", "role":"assistant", "turnId":"one", "generationStatus":"completed", "content":[{"type":"text", "text":"The final reply."}]},
            {"type":"reasoning", "turnId":"one", "text":"private thinking"},
            {"type":"effect", "turnId":"one", "output":"tool output"},
            {"type":"message", "role":"user", "turnId":"two", "content":[]},
            {"type":"message", "role":"assistant", "turnId":"two", "generationStatus":"completed", "content":[{"type":"text", "text":"Sure, reading it now."}]}
        ]});
        assert_eq!(
            previous_reply(&state, true).as_deref(),
            Some("The final reply.")
        );
        assert_eq!(
            previous_reply(&state, false).as_deref(),
            Some("Sure, reading it now.")
        );
        assert_eq!(
            previous_reply(&json!({"turns":[], "history":[]}), true),
            None
        );
    }
}
