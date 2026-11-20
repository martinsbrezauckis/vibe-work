//! Which browser each page reference (`ref`) of a chat came from, so a reference read in the person's own browser
//! (Your Chrome, Edge, Firefox, Safari) is never used on Vibe Work's built-in browser.
//!
//! Both browsers number their references the same way (`e00000002`), so one string can name different elements in
//! two browsers. Each chat keeps the references of its latest built-in read and of its latest read in its own
//! browser, each with the order of that read. A built-in action naming a reference is refused when that reference
//! came only from the person's browser, or when it came from both and the person's browser was read last; a
//! reference the built-in browser returned last, or one no read returned, goes on as before (the built-in browser
//! itself refuses an unknown or stale reference).
//!
//! Depends on: `chrome_control::Browser` (names). Called by: `agent_tools::browser_tools` (built-in
//! reads and actions) and `chrome_control::gateway` (reads in the person's browser).

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use serde_json::Value;
use vw_platform::sync::LockRecover;

use crate::chrome_control::Browser;

/// Chats whose references are kept (the oldest read is forgotten first).
const MAX_CHATS: usize = 256;
/// References kept per read (a read lists at most a few hundred).
const MAX_REFS: usize = 2048;

#[derive(Default)]
struct Read {
    order: u64,
    refs: BTreeSet<String>,
}

#[derive(Default)]
struct Chat {
    builtin: Read,
    own: Read,
    browser: Option<Browser>,
}

/// The per-chat record (one per core).
#[derive(Default)]
pub(crate) struct RefOrigins {
    chats: Mutex<(u64, HashMap<String, Chat>)>,
}

/// The `ref` strings anywhere in `value` (a read's answer or an action's arguments).
pub(crate) fn refs_in(value: &Value) -> BTreeSet<String> {
    fn walk(value: &Value, out: &mut BTreeSet<String>) {
        if out.len() >= MAX_REFS {
            return;
        }
        match value {
            Value::Object(fields) => {
                for (key, field) in fields {
                    match field {
                        Value::String(text) if key == "ref" => {
                            out.insert(text.clone());
                        }
                        other => walk(other, out),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(value, &mut out);
    out
}

impl RefOrigins {
    /// A read in the built-in browser (`own: None`) or in the person's browser returned `refs`; it replaces that
    /// browser's earlier references for this chat. A read that returned none changes nothing.
    pub(crate) fn note(&self, chat: &str, own: Option<Browser>, refs: BTreeSet<String>) {
        if refs.is_empty() {
            return;
        }
        let mut guard = self.chats.lock_or_recover();
        let (counter, chats) = &mut *guard;
        *counter += 1;
        let order = *counter;
        if !chats.contains_key(chat) && chats.len() >= MAX_CHATS {
            let oldest = chats
                .iter()
                .min_by_key(|(_, c)| c.builtin.order.max(c.own.order))
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                chats.remove(&oldest);
            }
        }
        let entry = chats.entry(chat.to_owned()).or_default();
        let read = Read { order, refs };
        match own {
            None => entry.builtin = read,
            Some(browser) => {
                entry.own = read;
                entry.browser = Some(browser);
            }
        }
    }

    /// Whether a built-in action naming `args`' references may run (module docs); the refusal names the browser.
    ///
    /// # Errors
    /// The refusal text for the agent.
    pub(crate) fn check_builtin(&self, chat: &str, args: &Value) -> Result<(), String> {
        let wanted = refs_in(args);
        if wanted.is_empty() {
            return Ok(());
        }
        let guard = self.chats.lock_or_recover();
        let Some(entry) = guard.1.get(chat) else {
            return Ok(());
        };
        let Some(browser) = entry.browser else {
            return Ok(());
        };
        let elsewhere = wanted.iter().find(|r| {
            entry.own.refs.contains(*r)
                && (!entry.builtin.refs.contains(*r) || entry.own.order > entry.builtin.order)
        });
        match elsewhere {
            None => Ok(()),
            Some(r) => Err(format!(
                "Not run: ref {r} came from your last read in {}, not from Vibe Work's browser. Act on that page \
                 with chrome_execute, or read Vibe Work's browser again and use one of its refs.",
                browser.name()
            )),
        }
    }
}

#[cfg(test)]
#[path = "browser_ref_origins_tests.rs"]
mod tests;
