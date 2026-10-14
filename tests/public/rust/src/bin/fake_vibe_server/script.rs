//! `--script` mode of the fake server: a small scripted Vibe with sessions, a turn that asks one
//! approval callback, and a state of record kept the same way the the fixture store reduces events, so a
//! test can compare the store's snapshot with this server's `session/read`.
//!
//! Turn script (`turn/start`, `turn.rs`): reply `{turn}` (status `in_progress`, Vibe's `PublicTurnStatus`); then, from a worker thread, events 1..:
//! user entry (id = the `clientUserMessageId`), `turn/started`, a callback entry plus the
//! `callback/call` server request (synthetic approval choices), wait up to 30 s for
//! `callback/result`, the callback entry resolved by `history/entryUpdated`, an assistant entry
//! (`approved` or `denied` + the decision), `turn/completed`.
//! Other methods: `session/start|resume|fork|read|list|delete|rewind`, `session/history/list` (`history.rs`), `session/history/clear` and a rewind into a
//! new session (both hand off to a new id, `handoff.rs`), `runtime/read`,
//! `session/agent/update`, `config/model/write`; anything else answers `{"echo": method}`.
//! `--status-events` makes the session status follow the turn (`session/updated` patches on
//! `/status`: running -> blocked while the approval is open -> running -> idle), the way the real
//! server reports it, so a UI can show the badge (the fixture). Off by default: it adds events, and
//! the the fixture/the fixture tests count event ids.
//! Session metadata (the fixture): `--seed FILE` loads a JSON array of `PublicSession` objects as
//! stored sessions (a seed's extra `history` array becomes its stored history, the fixture). `session/list` honours `limit`, `cursor` (an offset) and `includeArchived`, sorts
//! newest first and returns `continueSessionId`; `session/pin|archive|rename|markAsSeen` change the
//! stored session and publish the `session/updated` patch Vibe publishes; finishing a turn marks
//! the session unseen (`isUnseen: true`) when `--status-events` is on.
//!
//! Connections: `0` is a single-mode connection; `1..` are channels (`--multiplex`, `mux.rs`).
//! Multiplex rules for the scripted `--multiplex` mode:
//! - each channel holds at most one session (the one it started or resumed); its events and callbacks
//!   go to that channel only; a session held by no channel sends nothing;
//! - `turn/start|steer|interrupt` and `callback/result` for a session this channel does not hold answer
//!   `not_found`;
//! - `session/resume` of a session held by another channel answers `conflict` (Vibe: `session_busy`);
//!   `session/start` on a channel that holds a never-run session drops that session (Vibe replaces it);
//! - `session/pin|archive|rename|markAsSeen` from a channel that does not hold a session another channel
//!   holds are applied without a notification (Vibe applies them to disk only);
//! - `session/fork` (default `attach: true`) moves the channel to the fork and releases the source;
//! - a resume runs in the profile its `agentConfig.agent` names, else the default `accept-edits`
//! - closing a channel ends its session's work (an open callback is cancelled, the turn finishes, the
//!   session is idle) and releases it; a never-run session is gone after its channel closed.
//!
//! Single mode, `--single-attached` (the fixture): Vibe 2.26.0's one attached session per stdio
//! connection: `session/start|resume` attach the session, and `turn/start|steer|interrupt` and
//! `callback/result` for any other session fail with `not_found`; turns already running keep running.
//! Resuming a session that has an open callback re-delivers the callback (`callback/call`, id
//! `redeliver-<callback id>`) shortly after the response, like Vibe's `_redeliver_open_callbacks`; a
//! `callback/result` that arrives before the client acknowledged the re-delivery is refused
//! (`internal_error`). `session/markAsSeen|rename|pin|archive` for a session started or resumed here but
//! no longer attached answer success and change nothing; nothing is sent about a session that is not
//! attached. `--turn-delay-ms N` waits N ms before a turn's first event. `--rich-turn` (the fixture): every
//! turn plays the richer script of `rich.rs` (steps, an edit approval with a diff, a question).
//! `--log-context PATH` (the fixture) appends `{sessionId, context, user}` for each queued item with context entries.
//! `--log-methods PATH` appends every received request method (one per line), so a test can
//! prove a refused call never reached the server; `--log-channels PATH` appends
//! `<connection> <method> <sessionId|->`. `--bypass-profile NAME`: after switching to NAME,
//! `runtime/read` reports `bypassToolPermissions: true`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

use crate::meta::now_ms;

/// Writes one frame to one connection.
pub type Send = Arc<dyn Fn(Value) + std::marker::Send + Sync>;

pub use crate::meta::Options;

#[derive(Default)]
pub struct World {
    pub sessions: BTreeMap<String, Value>,
    pub next_id: u64,
    /// callback id -> the output the client sent.
    pub answers: BTreeMap<String, Value>,
    pub agent: BTreeMap<String, String>,
    /// connection -> the session it holds (attached).
    pub attached: BTreeMap<u64, String>,
    /// Callback ids whose re-delivery the client has not acknowledged yet.
    pub redelivering: BTreeSet<String>,
    /// Sessions started here that never ran a turn: Vibe has not stored them, so they are gone when
    /// their channel closes or the process dies (not persisted).
    pub unstored: BTreeSet<String>,
    /// After a restart from `--persist`: per session, the turn that was waiting on an approval when the
    /// old process died (finished on resume, see `resume_or_read`).
    pub pending_tail: BTreeMap<String, String>,
    /// Sessions started or resumed through the single connection (held "in memory").
    live: BTreeSet<String>,
    /// The turn queue per session and the running turn per session (`composer.rs`, the fixture).
    pub queues: BTreeMap<String, Vec<Value>>,
    pub running: BTreeMap<String, String>,
    /// Worktrees made for sessions (`worktrees.rs`, the fixture).
    pub claims: crate::worktrees::Claims,
    /// Sessions whose worktree is still being made (their turns wait, like Vibe's `defer_turns`).
    pub raising: BTreeSet<String>,
    /// Turns started from a queued item with context entries (the fixture: voice turns answer with Markdown).
    pub voice_turns: BTreeSet<String>,
    /// Parent mapping used only by the extended fixture runner.
    #[cfg(feature = "fixture-extensions")]
    pub child_parent: BTreeMap<String, String>,
    /// `--subagents-rule` (`subagents.rs`): the saved `enable_subagents` value (`None`: the default, on).
    pub subagents: Option<bool>,
    #[cfg(feature = "fixture-extensions")]
    pub extension_sessions: BTreeSet<String>,
}

impl World {
    /// The connection holding `sid`.
    pub fn owner(&self, sid: &str) -> Option<u64> {
        self.attached
            .iter()
            .find(|(_, held)| held.as_str() == sid)
            .map(|(conn, _)| *conn)
    }
}

/// The scripted server.
pub struct Script {
    pub world: Mutex<World>,
    pub answered: Condvar,
    sends: Mutex<BTreeMap<u64, Send>>,
    log: Option<Mutex<std::fs::File>>,
    channel_log: Option<Mutex<std::fs::File>>,
    pub bypass_profile: Option<String>,
    /// `--persist PATH`: the sessions are kept in this file across processes.
    pub persist: Option<String>,
    /// `--agent-update-fault MODE:N`: the first N `session/agent/update` calls fail (`fail`) or
    /// report another profile (`wrong`).
    pub agent_fault: Mutex<Option<(String, u32)>>,
    /// `--stuck-resumes N`: after a restart from `--persist`, the first N resumes of a session that was
    /// waiting on an approval never answer (synthetic timeout injection).
    pub stuck_resumes: Mutex<u32>,
    pub status_events: bool,
    #[cfg(feature = "fixture-extensions")]
    pub long_notification_error: bool,
    pub unstored_metadata_miss: bool,
    pub single_attached: bool,
    pub turn_delay_ms: u64,
    pub multiplex: bool,
    /// `--edit-files`: approved turns apply `edit:<path>` tokens to disk (`edits.rs`, the fixture).
    pub edit_files: bool,
    /// `--rich-turn`: turns play `rich.rs` (steps, diff, approval, question) instead of the plain script.
    pub rich_turn: bool,
    #[cfg(feature = "fixture-extensions")]
    pub extended_turn: bool,
    /// `--subagents-rule` (`subagents.rs`), and `--subagents-reject`.
    pub subagents_rule: bool,
    pub subagents_reject: bool,
    /// `--delete-faults N`: how many more `session/delete` calls fail.
    pub delete_faults: Mutex<u32>,
    /// `--log-context PATH` (the fixture): queued items with context entries, one JSON line each.
    context_log: Option<Mutex<std::fs::File>>,
    /// Optional model behavior used only by the composer regression.
    pub models: Option<crate::models::Models>,
}

pub fn fresh_state(id: &str, created: u64) -> Value {
    json!({
        "session": {"id": id, "title": null, "createdAt": created, "model": null,
                    "status": "idle", "cwd": "/fake/work"},
        "history": [],
        "turns": [],
        "activeCallbacks": [],
        "turnQueue": null,
        "childSessions": [],
        "eventId": 0,
    })
}

fn append_file(path: Option<String>) -> Option<Mutex<std::fs::File>> {
    path.map(|p| {
        Mutex::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .unwrap(),
        )
    })
}

impl Script {
    pub fn new(options: Options) -> Arc<Self> {
        let mut world = World::default();
        let persisted = options
            .persist
            .as_deref()
            .and_then(crate::meta::load_persisted);
        if let Some((sessions, pending)) = persisted {
            world.sessions = sessions;
            world.pending_tail = pending;
        } else if let Some(path) = options.seed_path {
            let text = std::fs::read_to_string(&path).expect("seed file");
            let items: Vec<Value> = serde_json::from_str(&text).expect("seed json");
            for mut session in items {
                let id = session["id"].as_str().expect("seed id").to_owned();
                let mut state = fresh_state(&id, 1);
                // the fixture: a seed may carry the stored history (`history`, not a `PublicSession` field).
                if let Some(history) = session.as_object_mut().and_then(|o| o.remove("history")) {
                    state["history"] = history;
                }
                state["session"] = session;
                world.sessions.insert(id, state);
            }
        }
        Arc::new(Self {
            world: Mutex::new(world),
            answered: Condvar::new(),
            sends: Mutex::new(BTreeMap::new()),
            log: append_file(options.log_path),
            channel_log: append_file(options.channel_log_path),
            bypass_profile: options.bypass_profile,
            persist: options.persist,
            stuck_resumes: Mutex::new(options.stuck_resumes),
            agent_fault: Mutex::new(options.agent_fault.and_then(|spec| {
                let (mode, count) = spec.split_once(':')?;
                Some((mode.to_owned(), count.parse().ok()?))
            })),
            status_events: options.status_events,
            #[cfg(feature = "fixture-extensions")]
            long_notification_error: options.long_notification_error,
            unstored_metadata_miss: options.unstored_metadata_miss,
            single_attached: options.single_attached,
            turn_delay_ms: options.turn_delay_ms,
            multiplex: options.multiplex,
            edit_files: options.edit_files,
            rich_turn: options.rich_turn,
            #[cfg(feature = "fixture-extensions")]
            extended_turn: options.extended_turn,
            subagents_rule: options.subagents_rule,
            subagents_reject: options.subagents_reject,
            delete_faults: Mutex::new(options.delete_faults),
            context_log: append_file(options.context_log_path),
            models: options.model_config_path.as_deref().map(|path| {
                crate::models::Models::from_path(path, options.model_log_path.as_deref())
            }),
        })
    }

    /// A connection opened (the single connection, or a channel).
    pub fn connect(&self, conn: u64, send: Send) {
        self.sends.lock().unwrap().insert(conn, send);
    }

    /// A channel closed: its session's work ends (open callbacks cancelled) and the session is
    /// released; a never-run fixture session is gone.
    pub fn disconnect(&self, conn: u64) {
        self.sends.lock().unwrap().remove(&conn);
        let mut w = self.world.lock().unwrap();
        let Some(sid) = w.attached.remove(&conn) else {
            return;
        };
        let open: Vec<String> = w.sessions.get(&sid).map_or_else(Vec::new, |s| {
            s["activeCallbacks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["callbackId"].as_str().map(str::to_owned))
                .collect()
        });
        for cb in open {
            w.answers
                .entry(cb)
                .or_insert_with(|| json!({"decision": {"type": "cancelled"}}));
        }
        if w.unstored.remove(&sid) {
            w.sessions.remove(&sid);
        }
        drop(w);
        self.release_holds(&sid);
        self.answered.notify_all();
    }

    /// The send of connection `conn`, if open.
    pub fn send_to(&self, conn: u64) -> Option<Send> {
        self.sends.lock().unwrap().get(&conn).cloned()
    }

    pub fn next(&self, prefix: &str) -> String {
        let mut w = self.world.lock().unwrap();
        w.next_id += 1;
        format!("{prefix}-{}", w.next_id)
    }

    /// Records a received request method.
    pub fn log_method(&self, method: &str) {
        if let Some(log) = &self.log {
            let mut file = log.lock().unwrap();
            let _ = writeln!(file, "{method}");
            let _ = file.flush();
        }
    }

    /// Records a queued item's context entries (`--log-context`, the fixture): `{sessionId, context: [texts], user:
    /// [texts]}`, so a test can prove the voice instruction travelled as context and not inside the user's text.
    pub fn log_context(&self, sid: &str, entries: &Value) {
        let Some(log) = &self.context_log else { return };
        let texts = |role: &str| -> Vec<Value> {
            entries
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["role"] == role)
                .flat_map(|e| e["content"].as_array().cloned().unwrap_or_default())
                .filter_map(|b| b.get("text").cloned())
                .collect()
        };
        let context = texts("context");
        if context.is_empty() {
            return;
        }
        let line = json!({"sessionId": sid, "context": context, "user": texts("user")});
        let mut file = log.lock().unwrap();
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }

    /// Records where a request arrived (`--log-channels`).
    pub fn log_channel(&self, conn: u64, method: &str, params: &Value) {
        if let Some(log) = &self.channel_log {
            let sid = params["sessionId"]
                .as_str()
                .or_else(|| params["sourceSessionId"].as_str())
                .unwrap_or("-");
            let mut file = log.lock().unwrap();
            let agent = params
                .pointer("/agentConfig/agent")
                .or_else(|| params.get("agentName"))
                .and_then(Value::as_str)
                .unwrap_or("-");
            let _ = writeln!(file, "{conn} {method} {sid} {agent}");
            let _ = file.flush();
        }
    }

    /// A frame without `method`: the client's answer to our `callback/call` (an ack).
    pub fn on_response(&self, frame: &Value) {
        if let Some(cb) = frame["id"]
            .as_str()
            .and_then(|id| id.strip_prefix("redeliver-"))
        {
            self.world.lock().unwrap().redelivering.remove(cb);
        }
    }

    /// True when `conn` holds `sid` (single mode without `--single-attached`: always).
    pub fn holds(&self, conn: u64, sid: &str) -> bool {
        if !self.multiplex && !self.single_attached {
            return true;
        }
        self.world
            .lock()
            .unwrap()
            .attached
            .get(&conn)
            .map(String::as_str)
            == Some(sid)
    }

    /// Answers one request from `conn`; `None` means the answer is sent later (never in this script).
    pub fn handle(self: &Arc<Self>, conn: u64, method: &str, params: &Value) -> Option<Value> {
        let sid = params["sessionId"].as_str().unwrap_or_default().to_owned();
        let attached_only = matches!(
            method,
            "turn/start" | "turn/steer" | "turn/interrupt" | "callback/result"
        );
        // the fixture: a subagent's card is answered on its parent's channel (`child.rs`).
        #[cfg(feature = "fixture-extensions")]
        let child_answer = method == "callback/result" && self.holds_parent_of(conn, &sid);
        #[cfg(not(feature = "fixture-extensions"))]
        let child_answer = false;
        if attached_only && !self.holds(conn, &sid) && !child_answer {
            return Some(json!({"__error": "not_found"}));
        }
        let metadata = matches!(
            method,
            "session/markAsSeen" | "session/rename" | "session/pin" | "session/archive"
        );
        if metadata && self.single_attached && !self.multiplex {
            let world = self.world.lock().unwrap();
            if world.live.contains(&sid) && world.attached.get(&conn) != Some(&sid) {
                return Some(json!({}));
            }
        }
        // Multiplex: a write from a channel that does not hold a session another channel holds is
        // applied without telling anyone (Vibe writes it to disk only).
        let notify = !self.multiplex || {
            let owner = self.world.lock().unwrap().owner(&sid);
            owner.is_none_or(|o| o == conn)
        };
        Some(match method {
            "session/start" => {
                let result = self.start(conn, params);
                self.raise_worktree(&result, params);
                result
            }
            m if crate::worktrees::HANDLES.contains(&m) => self.worktrees_call(conn, m, params),
            "session/backgroundProcess/output" => crate::background::output(params),
            // the fixture: stored history, paged backward (`history.rs`).
            "session/history/list" => {
                return Some(crate::history::list(self.state(&sid).as_ref(), params));
            }
            "session/resume" | "session/read" => {
                return self.resume_or_read(conn, method, &sid, params);
            }
            "session/fork" => {
                let source = params["sourceSessionId"].as_str().unwrap_or_default();
                let id = self.next("fake-fork");
                let mut state = self.state(source).unwrap_or_else(|| fresh_state(&id, 1));
                state["session"]["id"] = json!(id);
                let mut world = self.world.lock().unwrap();
                world.sessions.insert(id.clone(), state.clone());
                // Multiplex: a fork attaches by default (Vibe `attach: true` -> `adopt`): the channel
                // now holds the fork and the source is released.
                let attach = params["attach"].as_bool().unwrap_or(true);
                if self.multiplex
                    && attach
                    && world.attached.get(&conn).map(String::as_str) == Some(source)
                {
                    world.attached.insert(conn, id.clone());
                }
                drop(world);
                json!({"state": state, "sourceSessionId": source, "lastEventId": state["eventId"]})
            }
            // the fixture: `/clear` hands off to a new id (`handoff.rs`).
            m if crate::handoff::HANDLES.contains(&m) => {
                return Some(self.handoff_call(method, params));
            }
            m if crate::composer::HANDLES.contains(&m) => {
                return self.composer_call(method, params);
            }
            // the fixture: plugins and skills (`market.rs`).
            m if crate::market::HANDLES.contains(&m) => {
                return Some(crate::market::call(m, params));
            }
            // the fixture: connectors and MCP servers (`connectors.rs`).
            m if crate::connectors::HANDLES.contains(&m) => {
                return Some(self.connectors_call(conn, method, params));
            }
            "session/rewind" if params["inplace"] == json!(false) => self.rewind_into_new(params),
            "session/rewind" => match self.state(&sid) {
                Some(state) if self.edit_files => self.rewind_edits(&sid, state, params),
                Some(state) => json!({"state": state, "message": "rewound", "restoreErrors": [],
                    "restoredPaths": [], "sessionLog": {}}),
                None => return Some(json!({"__error": "not_found"})),
            },
            "session/list" => self.list(params),
            "session/pin" => {
                let pinned = params["pinned"].as_bool().unwrap_or(false);
                let at = if pinned { json!(now_ms()) } else { Value::Null };
                self.patch_fields(&sid, &[("pinnedAt", at.clone())], notify);
                json!({"pinnedAt": at})
            }
            "session/archive" => {
                let archived = params["archived"].as_bool().unwrap_or(false);
                let at = if archived {
                    json!(now_ms())
                } else {
                    Value::Null
                };
                // This fault mode acknowledges metadata without persisting it.
                if self.unstored_metadata_miss && self.world.lock().unwrap().unstored.contains(&sid)
                {
                    return Some(json!({"archivedAt": at}));
                }
                let mut fields = vec![("archivedAt", at.clone())];
                if archived {
                    fields.push(("pinnedAt", Value::Null));
                }
                self.patch_fields(&sid, &fields, notify);
                json!({"archivedAt": at})
            }
            "session/rename" => {
                let title = params["title"].clone();
                self.patch_fields(&sid, &[("title", title.clone())], notify);
                json!({"title": title, "lastEventId": null})
            }
            "session/markAsSeen" => {
                self.patch_fields(&sid, &[("isUnseen", json!(false))], notify);
                json!({})
            }
            "session/stop" => json!({}),
            "session/delete" => {
                {
                    let mut faults = self.delete_faults.lock().unwrap();
                    if *faults > 0 {
                        *faults -= 1;
                        return Some(json!({"__error": "internal_error"}));
                    }
                }
                let mut w = self.world.lock().unwrap();
                if self.unstored_metadata_miss && !w.sessions.contains_key(&sid) {
                    return Some(json!({"__error": "not_found"}));
                }
                if self.multiplex && w.owner(&sid).is_some() {
                    return Some(json!({"__error": "conflict"}));
                }
                w.sessions.remove(&sid);
                drop(w);
                self.release_holds(&sid);
                self.persist_now();
                json!({})
            }
            "turn/start" => {
                #[cfg(feature = "fixture-extensions")]
                if let Some(refusal) = self.fixture_subagents_turn(&sid) {
                    return Some(refusal);
                }
                if let Some(models) = &self.models {
                    models.log_turn(&sid);
                }
                // Vibe stores a session once it runs its first turn.
                self.world.lock().unwrap().unstored.remove(&sid);
                let turn_id = self.next("turn");
                let turn = json!({"id": turn_id, "status": "in_progress"});
                let this = self.clone();
                let params = params.clone();
                std::thread::spawn(move || this.run_turn(&sid, &turn_id, &params));
                json!({"turn": turn, "lastEventId": null})
            }
            "callback/result" => {
                let id = params["result"]["callbackId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if self.world.lock().unwrap().redelivering.contains(&id) {
                    return Some(json!({"__error": "internal_error"}));
                }
                let output = params["result"]["output"].clone();
                self.world.lock().unwrap().answers.insert(id, output);
                self.answered.notify_all();
                json!({})
            }
            "session/agent/update" => {
                self.agent_update(&sid, params["agentName"].as_str().unwrap_or_default())
            }
            "runtime/read" => {
                json!({"runtime": self.runtime(&sid), "ready": true, "sessionLog": {}})
            }
            "config/model/write" => {
                if let Some(models) = &self.models
                    && !models.write(&sid, params)
                {
                    return Some(json!({"__error": "invalid_params"}));
                }
                json!({"runtime": self.runtime(&sid)})
            }
            "config/read" if self.models.is_some() => {
                json!({"config": self.models.as_ref().map(|models| models.config(""))})
            }
            "config/write" if self.subagents_rule => self.config_write_subagents(&sid, params),
            "config/fields/read" if self.subagents_rule => self.config_fields_subagents(),
            "config/reload" => json!({"runtime": self.runtime(&sid)}),
            // Vibe's narrator summary (the fixture, `app_server/_narration.py`): one line that names what the
            // client sent, so a test sees the user text and the answer reached it.
            "narration/summarize" => json!({"summary": format!(
                "Fake summary of {:?} with {} characters of answer.",
                params["userMessage"].as_str().unwrap_or_default().chars().take(40).collect::<String>(),
                params["assistantText"].as_str().unwrap_or_default().chars().count()
            )}),
            other => json!({"echo": other}),
        })
    }

    /// `session/start`: a new session, held by `conn`. In multiplex mode a never-run session the
    /// channel held before is dropped (Vibe replaces it).
    fn start(&self, conn: u64, params: &Value) -> Value {
        let id = self.next("fake-session");
        let mut state = fresh_state(&id, 1_700_000_000_000);
        if let Some(cwd) = params.pointer("/agentConfig/cwd") {
            state["session"]["cwd"] = cwd.clone();
        }
        crate::background::add_to(&mut state);
        let mut world = self.world.lock().unwrap();
        if self.multiplex
            && let Some(old) = world.attached.get(&conn).cloned()
            && world.unstored.remove(&old)
        {
            world.sessions.remove(&old);
        }
        world.sessions.insert(id.clone(), state.clone());
        world.unstored.insert(id.clone());
        world.live.insert(id.clone());
        if let Some(agent) = params.pointer("/agentConfig/agent").and_then(Value::as_str) {
            world.agent.insert(id.clone(), agent.to_owned());
        }
        world.attached.insert(conn, id);
        drop(world);
        self.persist_now();
        json!({"state": state, "lastEventId": 0})
    }

    fn resume_or_read(
        self: &Arc<Self>,
        conn: u64,
        method: &str,
        sid: &str,
        params: &Value,
    ) -> Option<Value> {
        if self.state(sid).is_none() {
            #[cfg(feature = "fixture-extensions")]
            if let Some(failure) = self.fixture_resume_failure(method, sid) {
                return Some(failure);
            }
            return Some(json!({"__error": "not_found"}));
        }
        if method == "session/resume" && self.world.lock().unwrap().pending_tail.contains_key(sid) {
            let mut left = self.stuck_resumes.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return None; // never answered
            }
        }
        if method == "session/resume" {
            let pending = {
                let mut world = self.world.lock().unwrap();
                if self.multiplex && world.owner(sid).is_some_and(|o| o != conn) {
                    return Some(json!({"__error": "conflict"}));
                }
                world.attached.insert(conn, sid.to_owned());
                // (`accept-edits` in the test homes), not in the profile the session had.
                let agent = params
                    .pointer("/agentConfig/agent")
                    .and_then(Value::as_str)
                    .unwrap_or("accept-edits");
                world.agent.insert(sid.to_owned(), agent.to_owned());
                world.live.insert(sid.to_owned());
                world
                    .pending_tail
                    .remove(sid)
                    .map(|turn| (turn, agent.to_owned()))
            };
            // The persisted fixture finishes its pending step according to the resume profile.
            if let Some((turn, agent)) = pending {
                let decision = if matches!(agent.as_str(), "plan" | "ask") {
                    "deny"
                } else {
                    "approve"
                };
                self.finish_turn(sid, &turn, decision);
            }
            let state = self.state(sid).unwrap_or_default();
            // Multiplex: a channel's callbacks end with the channel, nothing to re-deliver.
            if self.single_attached && !self.multiplex {
                self.redeliver_open_callbacks(conn, &state);
            }
            return Some(json!({"state": state, "lastEventId": state["eventId"]}));
        }
        let state = self.state(sid).unwrap_or_default();
        // the fixture (Vibe 2.26.1): `--lease-holders FILE` (a JSON object session id -> pid, read on every read) is
        // the `leaseHolder` a `session/read` reports, like another process holding the session.
        let holder = crate::history::lease_holder(sid);
        Some(json!({"state": state, "lastEventId": state["eventId"], "leaseHolder": holder}))
    }

    pub fn state(&self, sid: &str) -> Option<Value> {
        self.world.lock().unwrap().sessions.get(sid).cloned()
    }
}
