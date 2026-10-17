//! The fake server's turn script and event emission (see `script.rs` for the protocol it plays).
//!
//! Events and `callback/call` requests go to the connection that holds the session: in multiplex mode
//! the channel that started or resumed it (none: nothing is sent); in single mode the one connection,
//! except with `--single-attached` for a session that is not attached (Vibe 2.26.0 sends nothing then).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::script::{Script, Send};

impl Script {
    /// Where frames about `sid` go right now (`None`: nobody hears about it). Call with the world
    /// locked (`w`).
    fn target(&self, w: &crate::script::World, sid: &str) -> Option<Send> {
        let conn = if self.multiplex {
            let owner = w.owner(sid);
            #[cfg(feature = "fixture-extensions")]
            let owner =
                owner.or_else(|| w.child_parent.get(sid).and_then(|parent| w.owner(parent)));
            owner?
        } else {
            if self.single_attached && w.attached.get(&0).map(String::as_str) != Some(sid) {
                return None;
            }
            0
        };
        self.send_to(conn)
    }

    /// [`Self::target`] with the world locked here.
    pub(crate) fn target_now(&self, sid: &str) -> Option<Send> {
        let w = self.world.lock().unwrap();
        self.target(&w, sid)
    }

    /// Applies `change` to the session's state, bumps its event id and sends the notification,
    /// all under the world lock so frames leave in event order.
    pub fn emit(
        &self,
        sid: &str,
        method: &str,
        mut params: Value,
        change: impl FnOnce(&mut Value),
    ) {
        let mut w = self.world.lock().unwrap();
        let target = self.target(&w, sid);
        let Some(state) = w.sessions.get_mut(sid) else {
            return;
        };
        change(state);
        let event_id = state["eventId"].as_u64().unwrap_or(0) + 1;
        state["eventId"] = json!(event_id);
        params["sessionId"] = json!(sid);
        params["eventId"] = json!(event_id);
        if let Some(send) = target {
            send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
        }
        drop(w);
        self.persist_now();
    }

    pub(crate) fn add_entry(&self, sid: &str, entry: Value) {
        let e = entry.clone();
        self.emit(
            sid,
            "history/entryAdded",
            json!({"entry": entry}),
            move |s| {
                s["history"].as_array_mut().unwrap().push(e.clone());
                if e["type"] == "callback" && e["state"]["status"] == "open" {
                    s["activeCallbacks"].as_array_mut().unwrap().push(e);
                }
            },
        );
    }

    /// With `--status-events`: patches the session's `/status` and tells the client.
    pub(crate) fn set_status(&self, sid: &str, status: Value) {
        if !self.status_events {
            return;
        }
        let new = status.clone();
        let finished = status["type"] == "idle";
        let mut patch = vec![json!({"op": "replace", "path": "/status", "value": status})];
        if finished {
            // Like Vibe: a session that finished work is unseen until someone opens it.
            patch.push(json!({"op": "add", "path": "/isUnseen", "value": true}));
        }
        self.emit(sid, "session/updated", json!({"patch": patch}), move |s| {
            s["session"]["status"] = new;
            if finished {
                s["session"]["isUnseen"] = json!(true);
            }
        });
    }

    pub(crate) fn upsert_turn(&self, sid: &str, method: &str, turn: Value) {
        let t = turn.clone();
        self.emit(sid, method, json!({"turn": turn}), move |s| {
            let turns = s["turns"].as_array_mut().unwrap();
            match turns.iter_mut().find(|x| x["id"] == t["id"]) {
                Some(slot) => *slot = t,
                None => turns.push(t),
            }
        });
    }

    pub fn run_turn(self: &Arc<Self>, sid: &str, turn_id: &str, params: &Value) {
        // Vibe holds a session's turns behind its worktree setup (`defer_turns`, the fixture).
        self.wait_for_worktree(sid);
        if self.turn_delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.turn_delay_ms));
        }
        let user_id = params["clientUserMessageId"]
            .as_str()
            .unwrap_or("user")
            .to_owned();
        let mut user_entry = json!({"id": user_id, "type": "message", "role": "user", "turnId": turn_id,
            "content": params["message"], "generationStatus": "completed"});
        // A queued item's display content (the fixture: the voice marker) is kept like Vibe keeps it.
        if params["userDisplayContent"].is_object() {
            user_entry["userDisplayContent"] = params["userDisplayContent"].clone();
        }
        if params["voice"] == true {
            self.world
                .lock()
                .unwrap()
                .voice_turns
                .insert(turn_id.to_owned());
        }
        self.add_entry(sid, user_entry);
        self.upsert_turn(
            sid,
            "turn/started",
            json!({"id": turn_id, "status": "in_progress"}),
        );
        self.world
            .lock()
            .unwrap()
            .running
            .insert(sid.to_owned(), turn_id.to_owned());
        self.set_status(sid, json!({"type": "running", "activeTurnId": turn_id}));
        #[cfg(feature = "fixture-extensions")]
        if self.maybe_workflow_stream(sid, turn_id, params) {
            return;
        }
        #[cfg(feature = "fixture-extensions")]
        if self.extended_turn {
            self.run_extended_turn(sid, turn_id);
            return;
        }
        // the fixture: a turn may hand the conversation to a new id and continue as it (`handoff.rs`).
        let continued = self.maybe_hand_off_in_turn(sid, turn_id, params);
        let sid = continued.as_str();
        self.maybe_web_search(sid, turn_id, params);
        self.maybe_files_refs(sid, turn_id, params);
        if self.rich_turn {
            self.run_rich_turn(sid, turn_id);
            return;
        }
        #[cfg(feature = "fixture-extensions")]
        if self.maybe_bash_turn(sid, turn_id, params) {
            return;
        }
        #[cfg(feature = "fixture-extensions")]
        if self.maybe_child_edits_turn(sid, turn_id, params) {
            return;
        }
        #[cfg(feature = "fixture-extensions")]
        if self.maybe_child_card_turn(sid, turn_id, params) {
            return;
        }
        #[cfg(feature = "fixture-extensions")]
        if self.maybe_mode_child_turn(sid, turn_id, params) {
            return;
        }
        // A profile that approves edits (switched to explicitly, or Vibe's resume default) runs the
        let unasked = matches!(
            self.world
                .lock()
                .unwrap()
                .agent
                .get(sid)
                .map(String::as_str),
            Some("accept-edits" | "auto-approve")
        );
        if unasked {
            self.apply_edits(sid, turn_id, params, "approve");
            self.finish_turn(sid, turn_id, "approve");
            self.run_next_queued(sid);
            return;
        }
        let cb = self.next("cb");
        let entry = json!({"id": cb, "callbackId": cb, "sessionId": sid, "turnId": turn_id,
            "type": "callback", "title": "Approve file_system.write_file",
            "generationStatus": "in_progress", "createdAt": 1, "updatedAt": 1,
            "detail": {"kind": "approval", "effect": {"toolName": "file_system.write_file"},
                "requiredPermissions": [],
                "choices": ["approve", "approve_for_session", "approve_permanently", "deny", "cancel_turn"],
                "pathScopeChoices": [], "reason": null},
            "state": {"status": "open"}});
        self.add_entry(sid, entry.clone());
        self.set_status(
            sid,
            json!({"type": "blocked", "activeTurnId": turn_id, "callbackId": cb,
                   "reason": "approval"}),
        );
        let target = {
            let w = self.world.lock().unwrap();
            self.target(&w, sid)
        };
        if let Some(send) = target {
            send(
                json!({"jsonrpc": "2.0", "id": format!("srv-{cb}"), "method": "callback/call",
                "params": {"callback": entry}}),
            );
        }
        let output = self.wait_answer(&cb, Duration::from_secs(30));
        // `turn/interrupt` (composer.rs) answers the open callback with this marker.
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
        // Vibe's `AnsweredCallbackState` (`models.py:921`): the answer as the client sent it.
        let resolved = json!({"status": "answered",
            "output": output.clone().unwrap_or_else(|| json!({"type": "approval", "decision": {"type": decision}}))});
        let r = resolved.clone();
        let cb_id = cb.clone();
        self.emit(
            sid,
            "history/entryUpdated",
            json!({"entryId": cb, "patch": [{"op": "replace", "path": "/state", "value": resolved}]}),
            move |s| {
                for e in s["history"].as_array_mut().unwrap() {
                    if e["id"] == cb_id.as_str() {
                        e["state"] = r.clone();
                    }
                }
                s["activeCallbacks"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|c| c["callbackId"] != cb_id.as_str());
            },
        );
        self.set_status(sid, json!({"type": "running", "activeTurnId": turn_id}));
        self.apply_edits(sid, turn_id, params, &decision);
        self.finish_turn(sid, turn_id, &decision);
        self.run_next_queued(sid);
    }

    /// The end of a turn: the assistant entry (`approved` or `denied` + the decision), `turn/completed`
    /// and (with `--status-events`) idle.
    pub fn finish_turn(&self, sid: &str, turn_id: &str, decision: &str) {
        let word = if decision.starts_with("approve") {
            "approved"
        } else {
            "denied"
        };
        let answer = self.next("assistant");
        // the fixture: a turn that carried context entries (a voice turn) answers with Markdown and a code block, so the
        // e2e can see what voice chat turns into speech.
        let voice = self.world.lock().unwrap().voice_turns.contains(turn_id);
        let text = if voice {
            format!(
                "**Done** ({word}). I wrote it:\n```js\nconst x = 1;\n```\nCheck `notes_v2.txt` next."
            )
        } else {
            format!("{word}: {decision}")
        };
        self.add_entry(
            sid,
            json!({"id": answer, "type": "message", "role": "assistant", "turnId": turn_id,
            "content": [{"type": "text", "text": text}],
            "generationStatus": "completed"}),
        );
        self.world.lock().unwrap().running.remove(sid);
        self.upsert_turn(
            sid,
            "turn/completed",
            json!({"id": turn_id, "status": "completed"}),
        );
        // the fixture: like Vibe, report the session's token counts and context fill after a turn (the status bar's
        // "Context 42%" and Settings › Usage and plan read them). With `--status-events` only, like the other extra
        // events: the the fixture/the fixture tests count event ids.
        if self.status_events {
            self.emit(
                sid,
                "session/statsUpdated",
                json!({"stats": {"sessionPromptTokens": 30_000, "sessionCompletionTokens": 12_000,
                "sessionTotalLlmTokens": 42_000, "contextTokens": 42_000}, "contextWindow": 100_000,
                "emittedAt": 0}),
                |_| {},
            );
        }
        self.set_status(sid, json!({"type": "idle"}));
    }

    pub(crate) fn wait_answer(&self, cb: &str, limit: Duration) -> Option<Value> {
        let started = Instant::now();
        let mut w = self.world.lock().unwrap();
        loop {
            if let Some(output) = w.answers.get(cb) {
                return Some(output.clone());
            }
            let left = limit.checked_sub(started.elapsed())?;
            w = self.answered.wait_timeout(w, left).unwrap().0;
        }
    }
}
