//! One approval per agent action: when the user approves a call once, the other approval
//! surface accepts that answer for the same call.
//!
//! The order of the two asks in Vibe's default runtime:
//! - **a `bash` call:** our `pre_tool` hook (the git guard `git_check`, the kill guard `kill_check`) runs FIRST; Vibe's
//!   approval card (`callback/call`, callback id `approval-<tool call id>`, the exact command in `effect.input`)
//!   comes after it, and only when the session's permissions ask (none in auto-approve, none when smart approve
//!   lets the call run, none when the user's config allows `bash`);
//! - **a call of our own MCP tools** (`mcp_vw.process_start`, `mcp_vw.secret_run`): Vibe's card comes FIRST (when the
//!   session asks for provided tools), then the call reaches the core, which runs the same guards inside the tool.
//!
//! So the core carries the user's one answer to the other ask of the same call:
//! - **guard first (bash):** a yes in the guard's native dialog is remembered for the exact call
//!   ([`Ledger::guard_said_yes`]: session, tool call id, command); when Vibe's card for that call arrives, the core
//!   answers it "approve" (once) itself and the card is not shown ([`Ledger::answer_for_card`], called by `pump`);
//! - **card first (our MCP tools):** a yes the user gave on Vibe's card is remembered for the exact call
//!   ([`Ledger::card_opened`] by `pump`, [`Ledger::card_answered`] by `dispatch`: session, tool, arguments); the guard
//!   inside the tool then does not ask again ([`Ledger::card_said_yes`]).
//!
//! Depends on: `serde_json`. Called by: `pump` (cards arriving), `dispatch` (the user's answers), `agent_tools`
//! (`gitcheck`, `killcheck`, `tools`, `secret_run`).
//! Invariants:
//! - Only an exact match counts: the same session, the same call (bash: the tool call id inside the card's id; MCP: the
//!   tool name and the whole argument object) and the same command text. Each memory is used once and forgotten after
//!   [`KEEP`]; at most [`MAX`] of each kind are kept (the oldest go first).
//! - Nothing here turns a refusal into a yes: a push is refused before the guards read the ledger, a declined dialog
//!   records nothing, and a card answered with anything but an approval records nothing.
//! - The answer the core sends for the user is "approve" (once) only, never "for this session" or "always" (those
//!   change Vibe's permission rules, which the user did not choose).
//! - A yes on Vibe's card only answers the guards' "may this exact command run" question (`card_said_yes`); it never
//!   stands in for a dialog that asks something else (giving a vault secret, an MCP key, a vault login fill: those
//!   stay the user's own answer in the native dialog, whoever answered the card).
//! - A card the core cannot answer is shown to the user (`pump`): a second ask is better than a stuck turn.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::command::Origin;

/// How long a remembered yes waits for the other ask of its call (Vibe's card follows the hook within ~0.1 s in the
/// measurements; a queue of earlier cards waiting on the user can delay it).
pub(crate) const KEEP: Duration = Duration::from_secs(10 * 60);

/// Most remembered items of each kind.
pub(crate) const MAX: usize = 256;

/// The prefix of Vibe's approval callback ids: `approval-<tool call id>` (`_unified_harness_backend_adapter`, measured
/// A different id never matches: the card is then shown (a double ask, never a missed one).
pub(crate) const CARD_PREFIX: &str = "approval-";

/// Who else asks the user about the call a guard is deciding.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Asked<'a> {
    /// Vibe's `bash` hook: Vibe's own card (if the session's permissions ask) comes AFTER the guard's answer. A yes in
    /// the guard's dialog is remembered for this call (`call_id` = the hook's `tool_call_id`; empty from an old hook),
    /// so the card is answered with it.
    Hook {
        /// The hook's `tool_call_id`.
        call_id: &'a str,
        /// The session id the hook named (a subagent's own id; the guard decides for its parent).
        hook_session: &'a str,
    },
    /// Our own tool (`process_start`, `secret_run`), which runs after Vibe's card for this exact call: `true` when the
    /// user approved that card (`Ledger::card_said_yes`), so the guard does not ask the same question again.
    Card(bool),
}

/// The answer the core sends on the user's behalf: Vibe's `ApprovalCallbackOutput` with "approve" (once).
#[must_use]
pub(crate) fn approve_once() -> Value {
    json!({"type": "approval", "decision": {"type": "approve"}})
}

struct GuardYes {
    sessions: Vec<String>,
    call_id: String,
    command: String,
    at: Instant,
}

struct Card {
    session: String,
    callback_id: String,
    tool: String,
    input: Value,
    at: Instant,
}

struct CardYes {
    session: String,
    tool: String,
    input: Value,
    /// Who answered the card: the window or the Debug API (which acts as the user outside the vault).
    by: Origin,
    at: Instant,
}

#[derive(Default)]
struct Inner {
    guard_yes: VecDeque<GuardYes>,
    cards: VecDeque<Card>,
    card_yes: VecDeque<CardYes>,
}

impl Inner {
    fn expire(&mut self, now: Instant) {
        self.guard_yes.retain(|g| now.duration_since(g.at) < KEEP);
        self.cards.retain(|c| now.duration_since(c.at) < KEEP);
        self.card_yes.retain(|c| now.duration_since(c.at) < KEEP);
    }
}

/// The remembered answers of one core (kept in the agent tools, `AgentTools::one_ask`: the guards exist only with them).
#[derive(Default)]
pub(crate) struct Ledger {
    inner: Mutex<Inner>,
}

/// The core's ledger, when the agent tools are on (no agent tools: no guards, nothing to carry).
pub(crate) fn of(shared: &crate::core::Shared) -> Option<&Ledger> {
    shared.agent_tools.as_deref().map(|tools| &tools.one_ask)
}

/// The parts of an approval callback entry (`callback/call` params' `callback`) the ledger reads.
fn approval_parts(callback: &Value) -> Option<(&str, &str, &Value)> {
    let detail = callback.get("detail")?;
    if detail.get("kind").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let session = callback.get("sessionId").and_then(Value::as_str)?;
    let id = callback
        .get("callbackId")
        .or_else(|| callback.get("id"))
        .and_then(Value::as_str)?;
    Some((session, id, detail.get("effect")?))
}

impl Ledger {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The user approved `command` in a guard's native dialog for the bash call `call_id` (the hook's
    /// `tool_call_id`). `sessions`: the hook's session id and the one the guard decided for (a subagent's call is
    /// decided for its parent); Vibe's card may name either.
    pub(crate) fn guard_said_yes(&self, sessions: &[&str], call_id: &str, command: &str) {
        if call_id.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut inner = self.lock();
        inner.expire(now);
        let mut names: Vec<String> = sessions
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| (*s).to_owned())
            .collect();
        names.dedup();
        inner.guard_yes.push_back(GuardYes {
            sessions: names,
            call_id: call_id.to_owned(),
            command: command.to_owned(),
            at: now,
        });
        while inner.guard_yes.len() > MAX {
            inner.guard_yes.pop_front();
        }
    }

    /// True when Vibe's approval card `callback` asks about a call whose guard dialog the user already approved: the
    /// same session, the callback id `approval-<that call id>`, the same command. The memory is used up.
    pub(crate) fn answer_for_card(&self, callback: &Value) -> bool {
        let Some((session, id, effect)) = approval_parts(callback) else {
            return false;
        };
        let Some(call_id) = id.strip_prefix(CARD_PREFIX) else {
            return false;
        };
        let Some(command) = effect.pointer("/input/command").and_then(Value::as_str) else {
            return false;
        };
        let now = Instant::now();
        let mut inner = self.lock();
        inner.expire(now);
        let found = inner.guard_yes.iter().position(|g| {
            g.call_id == call_id && g.command == command && g.sessions.iter().any(|s| s == session)
        });
        found.is_some_and(|at| inner.guard_yes.remove(at).is_some())
    }

    /// Remembers an approval card shown to the user (its session, id, tool and arguments), so the user's answer can
    /// be matched to the call it was about ([`Self::card_answered`]).
    pub(crate) fn card_opened(&self, callback: &Value) {
        let Some((session, id, effect)) = approval_parts(callback) else {
            return;
        };
        let Some(tool) = effect.get("toolName").and_then(Value::as_str) else {
            return;
        };
        let now = Instant::now();
        let mut inner = self.lock();
        inner.expire(now);
        inner.cards.push_back(Card {
            session: session.to_owned(),
            callback_id: id.to_owned(),
            tool: tool.to_owned(),
            input: effect.get("input").cloned().unwrap_or(Value::Null),
            at: now,
        });
        while inner.cards.len() > MAX {
            inner.cards.pop_front();
        }
    }

    /// The user answered card `callback_id` of `session` with `output` (sent to Vibe) from `by`: an approval
    /// ("approve", "for this session" or "always") is remembered for that exact call; anything else only forgets the
    /// card.
    pub(crate) fn card_answered(
        &self,
        session: &str,
        callback_id: &str,
        output: &Value,
        by: Origin,
    ) {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.expire(now);
        let Some(at) = inner
            .cards
            .iter()
            .position(|c| c.session == session && c.callback_id == callback_id)
        else {
            return;
        };
        let Some(card) = inner.cards.remove(at) else {
            return;
        };
        let approved = output.get("type").and_then(Value::as_str) == Some("approval")
            && output
                .pointer("/decision/type")
                .and_then(Value::as_str)
                .is_some_and(|d| d.starts_with("approve"));
        if approved {
            inner.card_yes.push_back(CardYes {
                session: card.session,
                tool: card.tool,
                input: card.input,
                by,
                at: now,
            });
            while inner.card_yes.len() > MAX {
                inner.card_yes.pop_front();
            }
        }
    }

    /// True when the user approved Vibe's card for this exact call of our own tool `tool` (`mcp_vw.<name>`) with
    /// `args` in `session` moments ago. The memory is used up.
    ///
    /// **Only for the guards' "may this exact command run" question** (the git guard and the kill guard inside
    /// `process_start` and `secret_run`), which is the question Vibe's card asks. A dialog that asks something the card
    /// does not (giving a vault secret to a program, an MCP key, filling a vault login: `secret_run`'s consent,
    /// `mcp_keys`, `browser_fill`, the confirmer's native-only methods) never reads this: it is always the user's own
    /// answer in the native dialog, also when the card was answered (from the window or the Debug API).
    pub(crate) fn card_said_yes(&self, session: &str, tool: &str, args: &Value) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.expire(now);
        let found = inner
            .card_yes
            .iter()
            .position(|c| c.session == session && c.tool == tool && &c.input == args);
        let Some(yes) = found.and_then(|at| inner.card_yes.remove(at)) else {
            return false;
        };
        tracing::info!(
            session,
            tool,
            by = yes.by.as_str(),
            "guard question already answered on Vibe's card (one ask per call)"
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Ledger, approve_once};
    use crate::command::Origin;
    use serde_json::{Value, json};

    fn card(session: &str, id: &str, tool: &str, input: &Value) -> Value {
        json!({"id": id, "callbackId": id, "sessionId": session, "type": "callback",
            "detail": {"kind": "approval", "effect": {"toolName": tool, "kind": "shell", "input": input}}})
    }

    #[test]
    fn a_guard_yes_answers_exactly_its_own_card_once() {
        let ledger = Ledger::default();
        let cmd = json!({"command": "git fetch"});
        // Nothing remembered: no card is answered for the user.
        assert!(!ledger.answer_for_card(&card("s", "approval-c1", "file_system.bash", &cmd)));
        ledger.guard_said_yes(&["s", "parent"], "c1", "git fetch");
        // Another call id, another session, another command, not an approval: no.
        assert!(!ledger.answer_for_card(&card("s", "approval-c2", "file_system.bash", &cmd)));
        assert!(!ledger.answer_for_card(&card("x", "approval-c1", "file_system.bash", &cmd)));
        assert!(!ledger.answer_for_card(&card(
            "s",
            "approval-c1",
            "file_system.bash",
            &json!({"command": "git fetch --all"})
        )));
        let mut question = card("s", "approval-c1", "file_system.bash", &cmd);
        question["detail"]["kind"] = json!("user_input");
        assert!(!ledger.answer_for_card(&question));
        // The parent session's card for the same call: yes, once.
        assert!(ledger.answer_for_card(&card("parent", "approval-c1", "file_system.bash", &cmd)));
        assert!(!ledger.answer_for_card(&card("parent", "approval-c1", "file_system.bash", &cmd)));
        // An empty call id (an old hook) is never remembered.
        ledger.guard_said_yes(&["s"], "", "git fetch");
        assert!(!ledger.answer_for_card(&card("s", "approval-", "file_system.bash", &cmd)));
        assert_eq!(approve_once()["decision"]["type"], "approve");
    }

    #[test]
    fn a_card_yes_counts_for_the_same_tool_call_only() {
        let ledger = Ledger::default();
        let args = json!({"command": "git fetch && npm run dev"});
        let tool = "mcp_vw.process_start";
        ledger.card_opened(&card("s", "approval-t1", tool, &args));
        ledger.card_opened(&card("s", "approval-t2", tool, &args));
        // Denied: nothing remembered.
        ledger.card_answered(
            "s",
            "approval-t1",
            &json!({"type": "approval", "decision": {"type": "deny"}}),
            Origin::Ui,
        );
        assert!(!ledger.card_said_yes("s", tool, &args));
        // An answer for an unknown card: nothing.
        ledger.card_answered("s", "approval-zz", &approve_once(), Origin::Ui);
        assert!(!ledger.card_said_yes("s", tool, &args));
        ledger.card_answered(
            "s",
            "approval-t2",
            &json!({"type": "approval", "decision": {"type": "approve_for_session"}}),
            Origin::DebugApi,
        );
        assert!(!ledger.card_said_yes("other", tool, &args));
        assert!(!ledger.card_said_yes("s", "mcp_vw.secret_run", &args));
        assert!(!ledger.card_said_yes("s", tool, &json!({"command": "git fetch"})));
        assert!(ledger.card_said_yes("s", tool, &args));
        assert!(
            !ledger.card_said_yes("s", tool, &args),
            "used once: the next call asks again"
        );
    }
}
