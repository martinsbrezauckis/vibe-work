//! `--rich-turn` (the fixture): a turn that shows every transcript surface the real-app screenshots need,
//! instead of the plain approval turn of `turn.rs`. Shapes follow Vibe 2.26.0's `PublicHistoryEntry`
//! invented (no capture data).
//!
//! Script after `turn/started` (status `running`):
//! 1. a reasoning entry, a completed `file_read` of `calc.py`, a completed `shell` run of the tests;
//! 2. a `file_edit` effect waiting for approval (`blocked`) and its approval callback (the card shows
//!    the edit as a diff); the session is `blocked` until `callback/result` (wait up to 120 s);
//! 3. on approval the edit completes with `occurrences` (the step shows the inline diff); a deny ends
//!    the turn like the plain script; an interrupt cancels it;
//! 4. a `user_input` question callback ("Which name should the helper get?", two options), blocked
//!    until answered (120 s);
//! 5. an assistant answer (markdown with a python block) naming the picked option, `turn/completed`,
//!    idle (with `--status-events`: unseen, which is what the OS notification follows).
//!
//! Called by: `turn.rs` (`run_turn`) when the script was started with `--rich-turn`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::script::Script;

/// How long the rich turn waits for each answer (the screenshot pass needs more than the plain 30 s).
const ANSWER_WAIT: Duration = Duration::from_secs(120);

const OLD_TEXT: &str = "def add(a, b):\n    return a + b";
const NEW_TEXT: &str =
    "def add(a, b):\n    return a + b\n\n\ndef multiply(a, b):\n    return a * b";

fn display(verb: &str, message: &str, status_text: &str) -> Value {
    json!({"summary": format!("{verb} {message}"), "content": null, "suffix": "", "verb": verb,
        "message": message, "settledVerb": format!("{verb}d"), "settledMessage": message,
        "statusText": status_text})
}

/// A completed effect's state; `verb` + `message` are the settled line ("Read" "calc.py").
fn settled_state(output: Value, output_text: &str, verb: &str, message: &str) -> Value {
    json!({"status": "completed", "outputText": output_text, "output": output, "durationMs": 1200,
        "approvalSource": "user",
        "display": {"success": true, "verb": verb, "message": message, "warnings": [],
            "approvalNote": null, "suffix": ""}})
}

impl Script {
    fn entry(&self, sid: &str, turn_id: &str, id: &str, kind: &str, extra: Value) -> Value {
        let now = crate::meta::now_ms();
        let mut entry = json!({"id": id, "sessionId": sid, "turnId": turn_id, "inputEntryId": null,
            "createdAt": now, "updatedAt": now, "generationStatus": "completed",
            "relatedEntryId": null, "type": kind});
        if let (Some(target), Some(fields)) = (entry.as_object_mut(), extra.as_object()) {
            for (key, value) in fields {
                target.insert(key.clone(), value.clone());
            }
        }
        entry
    }

    /// Replaces `/state` and `/generationStatus` of entry `id` (and drops it from the open
    /// callbacks when it was one).
    fn settle(&self, sid: &str, id: &str, state: Value) {
        let (s2, id2) = (state.clone(), id.to_owned());
        self.emit(
            sid,
            "history/entryUpdated",
            json!({"entryId": id, "patch": [
                {"op": "replace", "path": "/state", "value": state},
                {"op": "replace", "path": "/generationStatus", "value": "completed"}]}),
            move |s| {
                for e in s["history"].as_array_mut().unwrap() {
                    if e["id"] == id2.as_str() {
                        e["state"] = s2.clone();
                        e["generationStatus"] = json!("completed");
                    }
                }
                s["activeCallbacks"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|c| c["callbackId"] != id2.as_str());
            },
        );
    }

    /// Adds callback `entry`, blocks the session on it, sends `callback/call` and waits for the
    /// answer. `None`: no answer in time.
    fn ask(&self, sid: &str, turn_id: &str, entry: &Value, reason: &str) -> Option<Value> {
        let cb = entry["callbackId"].as_str().unwrap_or_default().to_owned();
        self.add_entry(sid, entry.clone());
        self.set_status(
            sid,
            json!({"type": "blocked", "activeTurnId": turn_id, "callbackId": cb, "reason": reason}),
        );
        if let Some(send) = self.target_now(sid) {
            send(
                json!({"jsonrpc": "2.0", "id": format!("srv-{cb}"), "method": "callback/call",
                "params": {"callback": entry}}),
            );
        }
        self.wait_answer(&cb, ANSWER_WAIT)
    }

    /// The `--rich-turn` script (see the module header). The user entry, `turn/started` and the
    /// running status were already sent by `run_turn`.
    pub fn run_rich_turn(self: &Arc<Self>, sid: &str, turn_id: &str) {
        let reasoning = self.next("reasoning");
        let text = "Reading calc.py and running the tests before changing anything.";
        self.add_entry(
            sid,
            self.entry(
                sid,
                turn_id,
                &reasoning,
                "reasoning",
                json!({"text": text, "summary": []}),
            ),
        );
        let read = self.next("effect");
        self.add_entry(sid, self.entry(sid, turn_id, &read, "effect", json!({
            "title": "read_file",
            "detail": {"toolName": "read_file", "display": display("Read", "calc.py", "Reading calc.py"),
                "kind": "file_read", "input": {"filePath": "calc.py"}},
            "state": settled_state(json!({"content": format!("{OLD_TEXT}\n")}), "", "Read", "2 lines from calc.py")})));
        let shell = self.next("effect");
        self.add_entry(sid, self.entry(sid, turn_id, &shell, "effect", json!({
            "title": "bash",
            "detail": {"toolName": "bash", "display": display("Run", "python -m pytest -q", "Running tests"),
                "kind": "shell", "input": {"command": "python -m pytest -q"}},
            "state": settled_state(json!({"stdout": "1 passed in 0.01s\n", "stderr": ""}), "1 passed in 0.01s\n", "Ran", "python -m pytest -q")})));

        // The edit waits for approval.
        let edit = self.next("effect");
        let edit_detail = json!({"toolName": "search_replace",
            "display": display("Edit", "calc.py", "Editing calc.py"), "kind": "file_edit",
            "input": {"filePath": "calc.py", "oldString": OLD_TEXT, "newString": NEW_TEXT}});
        let mut pending = self.entry(
            sid,
            turn_id,
            &edit,
            "effect",
            json!({"title": "search_replace", "detail": edit_detail.clone(),
                "state": {"status": "blocked", "outputText": ""}}),
        );
        pending["generationStatus"] = json!("in_progress");
        self.add_entry(sid, pending);
        let cb = self.next("cb");
        let mut approval = self.entry(sid, turn_id, &cb, "callback", json!({
            "callbackId": cb, "title": "Allow search_replace?", "relatedEntryId": edit,
            "detail": {"kind": "approval", "effect": edit_detail, "requiredPermissions": [],
                "choices": ["approve", "approve_for_session", "approve_permanently", "deny", "cancel_turn"],
                "pathScopeChoices": [], "relatedEntryId": edit, "reason": null},
            "state": {"status": "open"}}));
        approval["generationStatus"] = json!("in_progress");
        let output = self.ask(sid, turn_id, &approval, "approval");
        if output
            .as_ref()
            .is_some_and(|o| o.get("__interrupted").is_some())
        {
            self.interrupt_turn(sid, turn_id, &cb);
            return;
        }
        let decision = output
            .as_ref()
            .and_then(|o| o["decision"]["type"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "none".to_owned());
        self.settle(
            sid,
            &cb,
            json!({"status": "answered", "output": output.clone().unwrap_or_else(
                || json!({"type": "approval", "decision": {"type": decision}}))}),
        );
        self.set_status(sid, json!({"type": "running", "activeTurnId": turn_id}));
        if !decision.starts_with("approve") {
            self.finish_turn(sid, turn_id, &decision);
            self.run_next_queued(sid);
            return;
        }
        self.settle(
            sid,
            &edit,
            settled_state(
                json!({"file": "calc.py", "occurrences": [
                    {"startLine": 1, "oldText": OLD_TEXT, "newText": NEW_TEXT}]}),
                "",
                "Edited",
                "calc.py",
            ),
        );

        // A question.
        let q = self.next("cb");
        let mut question = self.entry(
            sid,
            turn_id,
            &q,
            "callback",
            json!({
            "callbackId": q, "title": "User input required",
            "detail": {"kind": "user_input", "request": {"questions": [{
                "question": "Which name should the new helper get?", "header": "Name",
                "options": [{"label": "multiply", "description": "matches add"},
                            {"label": "times", "description": "shorter"}],
                "multiSelect": false, "hideOther": false}],
                "footerNote": "The tests use the first one."}},
            "state": {"status": "open"}}),
        );
        question["generationStatus"] = json!("in_progress");
        let answer = self.ask(sid, turn_id, &question, "user_input");
        if answer
            .as_ref()
            .is_some_and(|o| o.get("__interrupted").is_some())
        {
            self.interrupt_turn(sid, turn_id, &q);
            return;
        }
        let picked = answer
            .as_ref()
            .and_then(|o| o.pointer("/result/answers/0/answer"))
            .and_then(Value::as_str)
            .unwrap_or("no answer")
            .to_owned();
        self.settle(
            sid,
            &q,
            json!({"status": "answered", "output": answer.unwrap_or_else(
                || json!({"type": "user_input", "result": {"answers": [], "cancelled": true}}))}),
        );
        self.set_status(sid, json!({"type": "running", "activeTurnId": turn_id}));
        let reply = self.next("assistant");
        let text = format!(
            "Added the helper to `calc.py` (you picked **{picked}**):\n\n```python\ndef multiply(a, b):\n    return a * b\n```\n\nThe tests still pass."
        );
        self.add_entry(
            sid,
            self.entry(
                sid,
                turn_id,
                &reply,
                "message",
                json!({"role": "assistant", "content": [{"type": "text", "text": text}]}),
            ),
        );
        self.world.lock().unwrap().running.remove(sid);
        self.upsert_turn(
            sid,
            "turn/completed",
            json!({"id": turn_id, "status": "completed"}),
        );
        self.set_status(sid, json!({"type": "idle"}));
        self.run_next_queued(sid);
    }
}
