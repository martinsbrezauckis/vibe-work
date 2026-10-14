//! The composer's methods in the fake server's `--script` mode (the fixture), so the key-free e2e
//! - `turn/steer`: a user entry with the steer's `clientUserMessageId` joins the running turn named by
//!   `expectedTurnId`; another id is a no-op (Vibe answers `stale_turn`).
//! - `session/turn/enqueue`: the item joins the session's queue (`turn/queueUpdated`, Vibe's event) and, when the
//!   session is idle, starts at once like Vibe (the fixture); its context entries are logged (`--log-context`);
//!   `queue/read|replace|remove` read and change it; when a turn finishes, the first queued item runs
//!   as the next turn (its user entry keeps the queued `entryId`, like Vibe).
//! - `turn/interrupt`: the running turn (named by `expectedTurnId`) has its open approval cancelled and
//!   ends `interrupted`; another id is a no-op.
//! - `session/shellCommand {command}`: a turnless `shell` effect; `echo X` prints `X`.
//! - `session/rewind/read`: no file changes; `skills/list`: one skill `review`; `session/compact`: the
//!   state with a compaction checkpoint (the e2e proves inserted context never reaches it).
//!
//! Vibe's shapes: `TurnEnqueueResponse {queueItemId}`, `PublicTurnQueue {items, paused, maxItems}`,
//! `SessionShellCommandResponse {accepted, lastEventId}` (`ui/src/protocol/generated`).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::meta::now_ms;
use crate::script::Script;

/// Methods this module answers.
pub const HANDLES: &[&str] = &[
    "turn/steer",
    "turn/interrupt",
    "session/turn/enqueue",
    "session/turn/queue/read",
    "session/turn/queue/replace",
    "session/turn/queue/remove",
    "session/shellCommand",
    "session/rewind/read",
    "skills/list",
    "session/compact",
];

impl Script {
    /// Answers one of [`HANDLES`].
    pub fn composer_call(self: &Arc<Self>, method: &str, params: &Value) -> Option<Value> {
        let sid = params["sessionId"].as_str().unwrap_or_default().to_owned();
        Some(match method {
            "turn/steer" => {
                let running = self.world.lock().unwrap().running.get(&sid).cloned();
                // Another id is a no-op here (Vibe: `stale_turn`); the core API command-table test
                // sends a made-up id and only needs the call to arrive.
                let Some(turn) = running.filter(|t| params["expectedTurnId"] == t.as_str()) else {
                    return Some(json!({"accepted": true}));
                };
                let id = params["clientUserMessageId"]
                    .as_str()
                    .unwrap_or("steer")
                    .to_owned();
                self.add_entry(
                    &sid,
                    json!({"id": id, "type": "message", "role": "user", "turnId": turn,
                        "content": params["message"], "generationStatus": "completed",
                        "source": "turn_steer", "createdAt": now_ms(), "updatedAt": now_ms()}),
                );
                json!({"accepted": true})
            }
            "turn/interrupt" => {
                // Only the turn the caller names (Vibe answers `stale_turn` otherwise; the core API
                // command-table test sends a made-up id and only needs the call to arrive).
                let running = self.world.lock().unwrap().running.get(&sid).cloned();
                if running.is_none_or(|t| params["expectedTurnId"] != t.as_str()) {
                    return Some(json!({}));
                }
                let open: Vec<String> = self
                    .state(&sid)
                    .and_then(|s| s["activeCallbacks"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|c| c["callbackId"].as_str().map(str::to_owned))
                    .collect();
                let mut world = self.world.lock().unwrap();
                for cb in open {
                    world.answers.insert(cb, json!({"__interrupted": true}));
                }
                drop(world);
                self.answered.notify_all();
                json!({})
            }
            "session/turn/enqueue" => {
                let id = self.next("queue");
                let item = json!({"id": id, "createdAt": now_ms(), "entries": params["entries"]});
                self.log_context(&sid, &params["entries"]);
                let idle = {
                    let mut world = self.world.lock().unwrap();
                    world.queues.entry(sid.clone()).or_default().push(item);
                    !world.running.contains_key(&sid)
                };
                self.publish_queue(&sid);
                // An idle synthetic session starts the queued item
                // at once.
                if idle {
                    self.run_next_queued(&sid);
                }
                json!({"queueItemId": id})
            }
            "session/turn/queue/read" => json!({"queue": self.queue_of(&sid)}),
            "session/turn/queue/replace" | "session/turn/queue/remove" => {
                let target = params["queueItemId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let mut world = self.world.lock().unwrap();
                let items = world.queues.entry(sid.clone()).or_default();
                let Some(at) = items.iter().position(|i| i["id"] == target.as_str()) else {
                    return Some(json!({"__error": "not_found"}));
                };
                if method.ends_with("remove") {
                    items.remove(at);
                } else {
                    items[at]["entries"] = params["entries"].clone();
                }
                drop(world);
                self.publish_queue(&sid);
                if method.ends_with("remove") {
                    json!({})
                } else {
                    json!({"queueItemId": target})
                }
            }
            "session/shellCommand" => {
                let command = params["command"].as_str().unwrap_or_default().to_owned();
                let printed = command.strip_prefix("echo ").unwrap_or("").to_owned();
                let id = self.next("shell");
                self.add_entry(
                    &sid,
                    json!({"id": id, "type": "effect", "turnId": null, "generationStatus": "completed",
                        "createdAt": now_ms(), "updatedAt": now_ms(),
                        "detail": {"kind": "shell", "input": {"command": command},
                            "display": {"summary": command, "verb": "Running", "message": command}},
                        "state": {"status": "completed", "output": {"stdout": format!("{printed}\n")},
                            "outputText": format!("{printed}\n"),
                            "display": {"success": true, "verb": "Ran", "message": command}}}),
                );
                let event = self
                    .state(&sid)
                    .map_or(0, |s| s["eventId"].as_u64().unwrap_or(0));
                json!({"accepted": true, "lastEventId": event})
            }
            "session/rewind/read" if self.edit_files => {
                let state = self.state(&sid).unwrap_or(Value::Null);
                let history = state["history"].as_array().cloned().unwrap_or_default();
                let cut = history
                    .iter()
                    .position(|entry| entry["id"] == params["entryId"])
                    .unwrap_or(history.len());
                let paths: Vec<_> = history[cut..]
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .pointer("/state/output/filePath")
                            .and_then(Value::as_str)
                    })
                    .collect();
                json!({"hasFileChanges": !paths.is_empty(), "paths": paths})
            }
            "session/rewind/read" => json!({"hasFileChanges": false, "paths": []}),
            "skills/list" => {
                json!({"skills": [{"name": "review", "description": "Review the changes",
                "prompt": "", "userInvocable": true, "enabled": true}]})
            }
            _ => {
                let state = self.state(&sid).unwrap_or_default();
                json!({"summary": "compacted", "state": state, "sessionLog": {}})
            }
        })
    }

    fn queue_of(&self, sid: &str) -> Value {
        let items = self
            .world
            .lock()
            .unwrap()
            .queues
            .get(sid)
            .cloned()
            .unwrap_or_default();
        json!({"items": items, "paused": false, "maxItems": 32})
    }

    fn publish_queue(&self, sid: &str) {
        let queue = self.queue_of(sid);
        let q = queue.clone();
        self.emit(
            sid,
            "turn/queueUpdated",
            json!({"queue": queue}),
            move |s| {
                s["turnQueue"] = q;
            },
        );
    }

    /// After a turn: starts the first queued item as the next turn (its user entry keeps the item's
    /// `entryId`).
    pub fn run_next_queued(self: &Arc<Self>, sid: &str) {
        let next = {
            let mut world = self.world.lock().unwrap();
            let items = world.queues.entry(sid.to_owned()).or_default();
            (!items.is_empty()).then(|| items.remove(0))
        };
        let Some(item) = next else { return };
        self.publish_queue(sid);
        let user = item["entries"]
            .as_array()
            .and_then(|e| e.iter().find(|x| x["role"] == "user").cloned())
            .unwrap_or_default();
        // The display content (`vibe.userDisplayContent`) goes with the user entry, as Vibe's promotion passes it;
        // `voice` marks an item that carried context entries (the fixture: the voice chat reply).
        let voice = item["entries"]
            .as_array()
            .is_some_and(|e| e.iter().any(|x| x["role"] == "context"));
        let params = json!({"clientUserMessageId": user["entryId"], "message": user["content"],
            "userDisplayContent": user["annotations"]["vibe.userDisplayContent"], "voice": voice});
        let turn_id = self.next("turn");
        // Marked running now (not only when the thread starts), so a second enqueue right after queues behind it.
        self.world
            .lock()
            .unwrap()
            .running
            .insert(sid.to_owned(), turn_id.clone());
        let this = Arc::clone(self);
        let sid = sid.to_owned();
        std::thread::spawn(move || this.run_turn(&sid, &turn_id, &params));
    }

    /// Ends a turn whose open approval `cb` was interrupted: the callback is cancelled, the turn ends
    /// `interrupted`, the session is idle.
    pub fn interrupt_turn(&self, sid: &str, turn_id: &str, cb: &str) {
        let cancelled = json!({"status": "cancelled", "reason": "Turn interrupted"});
        let c = cancelled.clone();
        let cb_id = cb.to_owned();
        self.emit(
            sid,
            "history/entryUpdated",
            json!({"entryId": cb, "patch": [{"op": "replace", "path": "/state", "value": cancelled}]}),
            move |s| {
                for e in s["history"].as_array_mut().unwrap() {
                    if e["id"] == cb_id.as_str() {
                        e["state"] = c.clone();
                    }
                }
                s["activeCallbacks"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|x| x["callbackId"] != cb_id.as_str());
            },
        );
        self.world.lock().unwrap().running.remove(sid);
        self.upsert_turn(
            sid,
            "turn/completed",
            json!({"id": turn_id, "status": "interrupted"}),
        );
        self.set_status(sid, json!({"type": "idle"}));
    }
}
