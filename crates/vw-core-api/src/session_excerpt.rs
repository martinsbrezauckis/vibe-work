//! A bounded excerpt of a session for a session mention (`sessionTools` op `excerpt`): its title, the first message
//! the person sent and the last answer. The composer puts it in the mention's chip, so the agent gets the other
//! session's gist without a tool, the way "Point Vibe at this" carries context.
//!
//! Read like the Markdown export reads (`session_export`): `session/history/list` pages through the gate, which reads
//! a stored session from the lobby without taking it; one short page from each end. Every text passes the vault-value
//! withholding chat titles use (`title_guard::withhold_text`) before it is cut, and each is cut to its bound with a
//! closing `…`: [`MAX_TITLE`] and [`MAX_MESSAGE`] characters (about 2 KB in all).
//!
//! Depends on: the gate, `names` (the title), `title_guard`. Called by: `session_tools`.
//! Invariants: read-only towards Vibe; no history text other than the two messages leaves this module.

use serde_json::{Value, json};

use crate::core::Core;
use crate::error::CoreError;
use crate::session_tools_view::SessionExcerptView;

/// Longest title kept (characters).
const MAX_TITLE: usize = 200;
/// Longest message kept (characters, before the closing `…`).
const MAX_MESSAGE: usize = 800;
/// Entries read from each end of the history.
const PAGE: u64 = 20;

/// `text` cut to `max` characters, with `…` when something was cut.
fn bounded(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{}…", kept.trim_end())
}

/// The text blocks of a message entry of `role`, joined; `None` for anything else or an empty message.
fn message_text(entry: &Value, role: &str) -> Option<String> {
    if entry["type"] != "message" || entry["role"] != role {
        return None;
    }
    let text: String = entry["content"]
        .as_array()?
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

impl Core {
    async fn history_end(
        &self,
        session_id: &str,
        direction: &str,
    ) -> Result<Vec<Value>, CoreError> {
        let reply = self
            .shared
            .gate
            .call(
                "session/history/list",
                json!({"sessionId": session_id, "page": {"limit": PAGE, "direction": direction}}),
                None,
            )
            .await
            .map_err(|e| CoreError::from_gate("session/history/list", &e))?;
        Ok(reply.result["items"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// The excerpt of `session_id` (see the module header).
    ///
    /// # Errors
    /// When Vibe cannot read the session's history.
    pub(crate) async fn session_excerpt(
        &self,
        session_id: &str,
    ) -> Result<SessionExcerptView, CoreError> {
        let shared = &self.shared;
        let withhold = |text: &str| crate::title_guard::withhold_text(shared, text);
        let first = self.history_end(session_id, "forward").await?;
        let last = self.history_end(session_id, "backward").await?;
        // A forward page lists the oldest entries first; a backward page holds the newest, also oldest first.
        let first_user = first.iter().find_map(|e| message_text(e, "user"));
        let last_answer = last.iter().rev().find_map(|e| message_text(e, "assistant"));
        let title = shared.names.display_title(&shared.store, session_id);
        Ok(SessionExcerptView {
            title: bounded(&withhold(&title), MAX_TITLE),
            first_user: first_user.map(|text| bounded(&withhold(&text), MAX_MESSAGE)),
            last_answer: last_answer.map(|text| bounded(&withhold(&text), MAX_MESSAGE)),
        })
    }
}
