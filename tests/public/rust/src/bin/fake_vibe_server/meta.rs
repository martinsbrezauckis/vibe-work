//! Session metadata, runtime and persistence of the scripted fake (`script.rs`): `session/list`,
//! metadata patches, callback re-delivery (single mode), the runtime snapshot with its active profile,
//! `session/agent/update` (with an optional fault), and `--persist` (sessions kept in a file across
//! processes, so a test can kill and restart the fake).
//!
//! - `session/agent/update` records the profile in the session (`session.agent.name`); a session
//!   started with `agentConfig.agent` has no such record;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::script::Script;

/// Settings from the command line.
pub struct Options {
    pub log_path: Option<String>,
    pub channel_log_path: Option<String>,
    pub bypass_profile: Option<String>,
    pub status_events: bool,
    #[cfg(feature = "fixture-extensions")]
    pub long_notification_error: bool,
    /// `--unstored-metadata-miss`: metadata calls acknowledge a never-run session without storing it.
    pub unstored_metadata_miss: bool,
    /// Persist archive metadata without publishing it, exercising the removal fallback read.
    pub archive_no_notify: bool,
    pub seed_path: Option<String>,
    pub single_attached: bool,
    pub turn_delay_ms: u64,
    pub multiplex: bool,
    pub persist: Option<String>,
    pub agent_fault: Option<String>,
    pub stuck_resumes: u32,
    pub edit_files: bool,
    pub rich_turn: bool,
    #[cfg(feature = "fixture-extensions")]
    pub extended_turn: bool,
    /// `--subagents-rule`: setting changes use a new helper chat (`subagents.rs`).
    pub subagents_rule: bool,
    /// `--subagents-reject`: with the rule on, `config/write` answers `rejected: true` and saves nothing.
    pub subagents_reject: bool,
    #[cfg(feature = "fixture-extensions")]
    /// `--proxy-echo-error`: `config/proxy/write` answers `invalid_params` with a text that repeats the first value sent
    /// (as Vibe's own errors do), so a test sees the core scrub it.
    pub proxy_echo_error: bool,
    /// `--delete-faults N`: the first N `session/delete` calls fail with `internal_error`.
    pub delete_faults: u32,
    /// `--log-context PATH` (the fixture): one JSON line per queued item that carries context entries.
    pub context_log_path: Option<String>,
    /// `--model-config PATH`: opt-in model profile read only from the test home's config.
    pub model_config_path: Option<String>,
    /// `--log-models PATH`: chosen model at each turn start.
    pub model_log_path: Option<String>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}

/// Sessions from a `--persist` file, as a restarted Vibe holds them: open callbacks gone, the session
/// idle, and per session the turn that was still `in_progress` (Vibe finishes its pending step on the
/// next resume, `Script::resume_or_read`, and then reports it `completed`).
pub fn load_persisted(path: &str) -> Option<(BTreeMap<String, Value>, BTreeMap<String, String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut sessions: BTreeMap<String, Value> = serde_json::from_str(&text).ok()?;
    let mut pending = BTreeMap::new();
    for (sid, state) in &mut sessions {
        let open = state["turns"].as_array().and_then(|turns| {
            turns
                .iter()
                .find(|t| t["status"] == "in_progress")
                .and_then(|t| t["id"].as_str().map(str::to_owned))
        });
        if let Some(turn) = open {
            pending.insert(sid.clone(), turn);
        }
        state["activeCallbacks"] = json!([]);
        state["session"]["status"] = json!({"type": "idle"});
    }
    Some((sessions, pending))
}

impl Script {
    /// Writes the sessions to the `--persist` file (no-op without it).
    pub fn persist_now(&self) {
        let Some(path) = &self.persist else {
            return;
        };
        // Under the world lock (writers from several threads), through a temporary file and a rename,
        // so a fake started after a kill never reads a half-written file.
        let world = self.world.lock().unwrap();
        let stored: BTreeMap<&String, &Value> = world
            .sessions
            .iter()
            .filter(|(id, _)| !world.unstored.contains(*id))
            .collect();
        let text = serde_json::to_string(&stored).unwrap();
        let tmp = format!("{path}.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
        drop(world);
    }

    /// `session/agent/update`: switches the profile, records it in the session, reports the runtime.
    /// With `--agent-update-fault`, the first calls fail or report another profile.
    pub fn agent_update(&self, sid: &str, name: &str) -> Value {
        {
            let mut fault = self.agent_fault.lock().unwrap();
            if let Some((mode, left)) = fault.as_mut().filter(|(_, left)| *left > 0) {
                *left -= 1;
                if mode == "fail" {
                    return json!({"__error": "internal_error"});
                }
                return json!({"runtime": {"activeAgent": {"name": "accept-edits"}}, "status": "applied"});
            }
        }
        {
            let mut world = self.world.lock().unwrap();
            world.agent.insert(sid.to_owned(), name.to_owned());
            if let Some(state) = world.sessions.get_mut(sid) {
                state["session"]["agent"] = json!({"name": name});
            }
        }
        self.persist_now();
        #[cfg(feature = "fixture-extensions")]
        self.reconfigure_mode_children(sid, name);
        json!({"runtime": self.runtime(sid), "status": "applied"})
    }

    /// `session/list`: archived hidden unless asked, newest `updatedAt` first, paged by offset.
    pub fn list(&self, params: &Value) -> Value {
        let w = self.world.lock().unwrap();
        let include = params["includeArchived"].as_bool().unwrap_or(false);
        let mut items: Vec<Value> = w
            .sessions
            .values()
            .map(|s| s["session"].clone())
            .filter(|s| include || s["archivedAt"].is_null())
            .collect();
        items.sort_by(|a, b| {
            b["updatedAt"]
                .as_i64()
                .cmp(&a["updatedAt"].as_i64())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let start: usize = params["cursor"]
            .as_str()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let limit = params["limit"].as_u64().unwrap_or(100) as usize;
        let page: Vec<Value> = items.iter().skip(start).take(limit).cloned().collect();
        let next = (start + page.len() < items.len()).then(|| (start + page.len()).to_string());
        json!({"items": page, "nextCursor": next,
               "continueSessionId": items.first().map(|s| s["id"].clone())})
    }

    /// Sets session fields and (when `notify`) publishes the `session/updated` patch Vibe publishes.
    pub fn patch_fields(&self, sid: &str, fields: &[(&str, Value)], notify: bool) {
        let owned: Vec<(String, Value)> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        let apply = move |s: &mut Value| {
            for (k, v) in owned {
                s["session"][k] = v;
            }
        };
        if !notify {
            if let Some(state) = self.world.lock().unwrap().sessions.get_mut(sid) {
                apply(state);
            }
            return;
        }
        let ops: Vec<Value> = fields
            .iter()
            .map(|(k, v)| json!({"op": "add", "path": format!("/{k}"), "value": v}))
            .collect();
        self.emit(sid, "session/updated", json!({"patch": ops}), apply);
    }

    /// Re-sends each open callback of a resumed session as a `callback/call` request, 30 ms after
    /// the resume response, and remembers it as unacknowledged until the client's answer arrives.
    pub fn redeliver_open_callbacks(self: &Arc<Self>, conn: u64, state: &Value) {
        let open: Vec<Value> = state["activeCallbacks"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for entry in open {
            let Some(cb) = entry["callbackId"].as_str().map(str::to_owned) else {
                continue;
            };
            self.world.lock().unwrap().redelivering.insert(cb.clone());
            let Some(send) = self.send_to(conn) else {
                continue;
            };
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                send(
                    json!({"jsonrpc": "2.0", "id": format!("redeliver-{cb}"), "method": "callback/call",
                    "params": {"callback": entry}}),
                );
            });
        }
    }

    pub fn runtime(&self, sid: &str) -> Value {
        let agent = self.world.lock().unwrap().agent.get(sid).cloned();
        let bypass = agent.is_some() && agent == self.bypass_profile;
        let active = agent.unwrap_or_else(|| "accept-edits".to_owned());
        let config = self.models.as_ref().map_or_else(
            || {
                json!({"models": [{"alias": "cli-fast", "name": "mistral-vibe-cli-fast"}],
                      "activeModel": {"alias": "cli-fast"}})
            },
            |models| models.config(sid),
        );
        json!({"config": config,
               "activeAgent": {"name": active},
               "bypassToolPermissions": bypass})
    }
}
