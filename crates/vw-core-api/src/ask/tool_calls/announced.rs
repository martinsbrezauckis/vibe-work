//! Proof that Vibe itself started the call a receipt names.
//!
//! A receipt is a single-use secret the core mints for Vibe's pre-tool hook. Vibe copies the hook's rewritten input,
//! receipt included, into the call's history entry before it sends the call to the chat's tool server. The core reads
//! that entry from its own connection to Vibe: pushed on the chat's channel (`history/entryAdded`), or, for a subagent
//! whose history is not pushed there, read back with `session/read`. The hook and chat credentials can be copied by a
//! same-user program on some systems, so they never prove where a call came from; only this entry does.
//!
//! Depends on: `channels` (`ChannelKey`). Called by: `receipts` (pump observation, the tool route's wait, `take`).
use serde_json::Value;

use super::names;
use crate::channels::ChannelKey;

/// How the core learned that Vibe carried a receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Pushed on this channel of the core's Vibe connection.
    Channel(ChannelKey),
    /// Read back from Vibe with `session/read`.
    Read,
    /// A debug-build test fixture standing in for Vibe (`Core::announce_tool_call_for_fixture`).
    #[cfg(any(test, debug_assertions))]
    Fixture,
}

/// The session whose history entry carried the receipt, and how the core saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Announcement {
    pub(crate) session: String,
    pub(crate) origin: Origin,
}

/// A running call entry of a Vibe Work tool: the session it belongs to, the tool (without `mcp_vw.`) and its receipt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Entry<'a> {
    pub(crate) session: &'a str,
    pub(crate) tool: &'a str,
    pub(crate) receipt: &'a str,
}

/// The call a history entry describes, when it is a pending or running effect of a Vibe Work tool that carries a
/// receipt. `session` is the session the entry is reported for; an entry naming a different session is refused.
pub(crate) fn entry<'a>(value: &'a Value, session: Option<&'a str>) -> Option<Entry<'a>> {
    if value.get("type").and_then(Value::as_str) != Some("effect")
        || !matches!(
            value.pointer("/state/status").and_then(Value::as_str),
            Some("pending" | "running" | "in_progress")
        )
    {
        return None;
    }
    let tool = value
        .pointer("/detail/toolName")
        .and_then(Value::as_str)?
        .strip_prefix("mcp_vw.")?;
    let receipt = value
        .pointer("/detail/input")?
        .get(names::RECEIPT_FIELD)?
        .as_str()
        .filter(|r| !r.is_empty())?;
    let named = value.get("sessionId").and_then(Value::as_str);
    let session = match (named, session) {
        (Some(named), Some(reported)) if named != reported => return None,
        (Some(named), _) => named,
        (None, Some(reported)) => reported,
        (None, None) => return None,
    };
    Some(Entry {
        session,
        tool,
        receipt,
    })
}

/// The call a Vibe notification announces: only a newly added history entry counts.
pub(crate) fn notification<'a>(method: &str, params: &'a Value) -> Option<Entry<'a>> {
    if method != "history/entryAdded" {
        return None;
    }
    entry(
        params.get("entry")?,
        params.get("sessionId").and_then(Value::as_str),
    )
}

/// Whether a `session/read` reply for `session` shows the running call of `tool` that carries `receipt`.
pub(crate) fn in_history(reply: &Value, session: &str, tool: &str, receipt: &str) -> bool {
    reply
        .pointer("/state/history")
        .and_then(Value::as_array)
        .is_some_and(|history| {
            history.iter().rev().any(|value| {
                entry(value, Some(session))
                    .is_some_and(|e| e.session == session && e.tool == tool && e.receipt == receipt)
            })
        })
}

/// The binding rule at the final dispatch. The entry must name the receipt's own source session. A pushed entry
/// must have arrived on the owner chat's live channel; a read-back entry must belong to the owner chat or to a
/// subagent the core's own ownership record puts under it (`is_child`, asked only then).
pub(crate) fn started_by_vibe(
    announcement: Option<&Announcement>,
    owner: &str,
    source: &str,
    owner_channel: Option<ChannelKey>,
    is_child: impl FnOnce() -> bool,
) -> bool {
    let Some(announcement) = announcement.filter(|a| a.session == source) else {
        return false;
    };
    let in_chat = || source == owner || is_child();
    match announcement.origin {
        Origin::Channel(key) => owner_channel == Some(key) && in_chat(),
        Origin::Read => in_chat(),
        #[cfg(any(test, debug_assertions))]
        Origin::Fixture => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn effect(session: &str, tool: &str, status: &str, receipt: &str) -> Value {
        json!({"type":"effect","id":"e1","sessionId":session,"state":{"status":status},
            "detail":{"toolName":tool,"input":{"command":"x","_vwReceipt":receipt}}})
    }

    #[test]
    fn only_an_added_running_effect_of_a_vibe_work_tool_announces_its_receipt() {
        let added = |entry: Value| json!({"sessionId":"chat-a","entry":entry});
        let running = added(effect("chat-a", "mcp_vw.secret_run", "running", "k1"));
        assert_eq!(
            notification("history/entryAdded", &running),
            Some(Entry {
                session: "chat-a",
                tool: "secret_run",
                receipt: "k1"
            })
        );
        assert_eq!(notification("history/entryUpdated", &running), None);
        for refused in [
            added(effect("chat-a", "mcp_vw.secret_run", "completed", "k1")),
            added(effect("chat-a", "mcp_vw.secret_run", "skipped", "k1")),
            added(effect("chat-a", "file_system.bash", "running", "k1")),
            added(effect("other", "mcp_vw.secret_run", "running", "k1")),
            added(effect("chat-a", "mcp_vw.secret_run", "running", "")),
            added(
                json!({"type":"message","sessionId":"chat-a","state":{"status":"running"},
                "detail":{"toolName":"mcp_vw.secret_run","input":{"_vwReceipt":"k1"}}}),
            ),
            // A receipt that only appears in the text of some other field is not an announcement.
            added(
                json!({"type":"effect","sessionId":"chat-a","state":{"status":"running"},
                "detail":{"toolName":"mcp_vw.secret_run","input":{"note":{"_vwReceipt":"k1"}}}}),
            ),
        ] {
            assert_eq!(
                notification("history/entryAdded", &refused),
                None,
                "{refused}"
            );
        }
    }

    #[test]
    fn a_read_back_history_shows_only_the_named_sessions_running_call() {
        let reply = json!({"state":{"history":[
            effect("child", "mcp_vw.process_start", "completed", "old"),
            effect("child", "mcp_vw.process_start", "running", "k1"),
        ]}});
        assert!(in_history(&reply, "child", "process_start", "k1"));
        assert!(!in_history(&reply, "child", "process_start", "old"));
        assert!(!in_history(&reply, "child", "secret_run", "k1"));
        assert!(!in_history(&reply, "parent", "process_start", "k1"));
        assert!(!in_history(&json!({}), "child", "process_start", "k1"));
    }

    #[test]
    fn the_entry_must_name_the_source_and_reach_the_owner_on_its_own_channel() {
        let seen = |session: &str, origin| Announcement {
            session: session.into(),
            origin,
        };
        let own = Some((1, 7));
        let pushed = seen("chat-a", Origin::Channel((1, 7)));
        assert!(started_by_vibe(
            Some(&pushed),
            "chat-a",
            "chat-a",
            own,
            || { false }
        ));
        assert!(!started_by_vibe(None, "chat-a", "chat-a", own, || true));
        // The receipt names another chat than the one Vibe reported.
        assert!(!started_by_vibe(
            Some(&pushed),
            "chat-a",
            "other",
            own,
            || true
        ));
        // Reported on another chat's channel, or the owner has no live channel.
        let elsewhere = seen("chat-a", Origin::Channel((1, 8)));
        assert!(!started_by_vibe(
            Some(&elsewhere),
            "chat-a",
            "chat-a",
            own,
            || true
        ));
        assert!(!started_by_vibe(
            Some(&pushed),
            "chat-a",
            "chat-a",
            None,
            || true
        ));
        // A subagent: read back from Vibe and owned by the chat, or not owned (a forged parent).
        let child = seen("child", Origin::Read);
        assert!(started_by_vibe(
            Some(&child),
            "chat-a",
            "child",
            own,
            || true
        ));
        assert!(!started_by_vibe(
            Some(&child),
            "chat-a",
            "child",
            own,
            || { false }
        ));
        let pushed_child = seen("child", Origin::Channel((1, 7)));
        assert!(!started_by_vibe(
            Some(&pushed_child),
            "chat-a",
            "child",
            own,
            || false
        ));
    }
}
