//! Fixture intent turned into real stored sessions by pinned Vibe and scripted model turns.
//! Depends on the real core and local model; callers receive captured IDs/folders, never fabricated protocol replies.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vw_core_api::{Command, CommandResult, Core, Origin};

fn fail(message: &str) -> ! {
    panic!("real session setup: {message}")
}

async fn run(core: &Core, command: Value) -> Result<CommandResult, String> {
    let parsed: Command =
        serde_json::from_value(command.clone()).map_err(|e| format!("{command}: {e}"))?;
    core.execute(Origin::Ui, parsed)
        .await
        .map_err(|e| format!("{}: {e:?}", command["type"]))
}

/// A generic call, with a confirmation token when the gate asks for one.
async fn call(core: &Core, method: &str, params: Value) -> Result<(), String> {
    let token = match run(
        core,
        json!({"type": "requestConfirmation", "method": method, "params": params}),
    )
    .await
    {
        Ok(CommandResult::Confirmation { token }) => Some(token),
        _ => None,
    };
    run(
        core,
        json!({"type": "call", "method": method, "params": params, "confirmationToken": token}),
    )
    .await
    .map(|_| ())
}

/// Waits until the chat is idle with its last turn ended (60 s).
async fn quiet(core: &Core, sid: &str, minimum_turns: usize) -> Result<(), String> {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(CommandResult::Session { snapshot, .. }) =
            run(core, json!({"type": "readSession", "sessionId": sid})).await
        {
            let state = snapshot.state;
            // startTurn can answer before its events reach the store. An earlier idle turn cannot satisfy this
            // wait: closing now would cancel the newly submitted turn and silently lose its assistant message.
            let ended = state["turns"].as_array().is_some_and(|turns| {
                turns.len() >= minimum_turns
                    && turns.last().is_some_and(|t| t["status"] == "completed")
            });
            if state["session"]["status"]["type"] == "idle" && ended {
                return Ok(());
            }
        }
        if Instant::now() > end {
            return Err(format!("seed: {sid} did not finish its turn"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ApproveAll;

impl vw_gate::Confirmer for ApproveAll {
    fn confirm(
        &self,
        _: &vw_gate::ConfirmationRequest,
    ) -> impl std::future::Future<Output = vw_gate::Confirmation> + Send {
        std::future::ready(vw_gate::Confirmation::Approved)
    }
}

/// A throwaway core owns setup only; the core under test starts after all its real sessions are closed.
async fn setup_core(
    dir: &Path,
    home: &Path,
    model: &super::scripted_model::ScriptedModel,
    children: bool,
) -> Core {
    let mut server = super::shared_real_vibe::supervisor_config(dir, model);
    super::shared_real_vibe::reuse_home(&mut server, home, model);
    if children {
        let path = home.join("config.toml");
        let mut config: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        config["enable_subagents"] = toml::Value::Boolean(true);
        std::fs::write(path, toml::to_string(&config).unwrap()).unwrap();
    }
    let mut config = vw_core_api::CoreConfig::default();
    config.sessions_dir = Some(dir.join("setup-core/sessions"));
    let secret = model.route(&mut server).await;
    Core::start(server, secret, ApproveAll, config)
        .await
        .unwrap()
}

/// Persist the requested history using the real process, then release every lease before returning its ID map.
pub async fn create(
    dir: &Path,
    home: &Path,
    model: &super::scripted_model::ScriptedModel,
    items: &[Value],
) -> Value {
    let input = dir.join("seed-input.json");
    std::fs::write(&input, serde_json::to_vec(items).unwrap()).unwrap();
    let children = items.iter().any(|s| s["parentSessionId"].is_string());
    let custom = items.iter().any(|s| s["agent"]["name"] == "custom");
    if custom {
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::write(
            home.join("agents/custom.toml"),
            "display_name = \"Fixture custom\"\n",
        )
        .unwrap();
    }
    let core = setup_core(dir, home, model, children).await;
    populate(&core, model, dir, &input).await;
    core.shutdown().await;
    let record: Value =
        serde_json::from_slice(&std::fs::read(dir.join("seed-ids.json")).unwrap()).unwrap();
    if custom {
        // A foreign CLI can select a custom profile that Vibe Work deliberately never offers.
        let mut server =
            super::shared_real_vibe::supervisor_config(&dir.join("custom-profile-cli"), model);
        super::shared_real_vibe::reuse_home(&mut server, home, model);
        let secret = model.route(&mut server).await;
        let cli = vw_supervisor::Supervisor::start(server, secret)
            .await
            .unwrap();
        for item in items
            .iter()
            .filter(|item| item["agent"]["name"] == "custom")
        {
            let sid = record["ids"][item["id"].as_str().unwrap()]
                .as_str()
                .unwrap();
            let channel = cli.open_channel(Duration::from_secs(60)).await.unwrap();
            channel
                .conn
                .request(
                    "session/resume",
                    json!({"sessionId":sid,"agentConfig":{"agent":"custom"}}),
                )
                .await
                .unwrap();
            channel
                .conn
                .request(
                    "session/agent/update",
                    json!({"sessionId":sid,"agentName":"custom"}),
                )
                .await
                .unwrap();
            channel.conn.close_input().await;
        }
        cli.shutdown().await;
    }
    if children {
        let metadata = setup_core(dir, home, model, false).await;
        for item in items.iter().filter(|s| s["parentSessionId"].is_string()) {
            let id = &record["ids"][item["id"].as_str().unwrap()];
            if item["title"].is_string() {
                call(
                    &metadata,
                    "session/rename",
                    json!({"sessionId":id,"title":item["title"]}),
                )
                .await
                .unwrap();
            }
        }
        metadata.shutdown().await;
    }
    record
}

async fn populate(
    core: &Core,
    model: &super::scripted_model::ScriptedModel,
    data_dir: &Path,
    file: &Path,
) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| fail(&format!("seed: {e}")));
    let mut sessions: Vec<Value> =
        serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("seed: {e}")));
    sessions.sort_by_key(|s| s["updatedAt"].as_i64().unwrap_or(0));
    // The model answers each stored user message with the assistant message that followed it.
    let mut answers: BTreeMap<String, String> = BTreeMap::new();
    for s in &sessions {
        let history = s["history"].as_array().cloned().unwrap_or_default();
        for pair in history.windows(2) {
            if pair[0]["role"] == "user" && pair[1]["role"] == "assistant" {
                answers.insert(
                    super::scripted_model::text_of(&pair[0]["content"]),
                    super::scripted_model::text_of(&pair[1]["content"]),
                );
            }
        }
    }
    model.set_hook(std::sync::Arc::new(move |s| {
        if !s.side() && s.step == 0 && s.last_user().starts_with("rv1-create-child:") {
            return Some(super::scripted_model::Reply {
                tools: vec![super::scripted_model::Call {
                    name: "run_typescript".to_owned(),
                    args: json!({"code":"async function main() { await tools.agents.spawn({name: 'seeded-child', message: 'Reply with the words seeded child, without using tools.'}); }"}),
                }],
                ..super::scripted_model::Reply::default()
            });
        }
        if !s.side() && s.step == 1 && s.last_user().starts_with("rv1-create-child:") {
            return Some(super::scripted_model::Reply {
                tools: vec![super::scripted_model::Call {
                    name: "wait".into(), args: json!({"timeout_ms":60000}),
                }],
                ..super::scripted_model::Reply::default()
            });
        }
        (!s.side() && s.step == 0).then(|| super::scripted_model::Reply {
            text: Some(
                answers
                    .get(s.last_user())
                    .cloned()
                    .unwrap_or_else(|| "ok".to_owned()),
            ),
            ..super::scripted_model::Reply::default()
        })
    }));
    let started = Instant::now();
    let mut ids = serde_json::Map::new();
    let mut folders = serde_json::Map::new();
    let mut unsupported = Vec::new();
    for s in &sessions {
        let seed_id = s["id"].as_str().unwrap_or_default().to_owned();
        if s["parentSessionId"].is_string() {
            continue;
        }
        let folder: PathBuf = match s["cwd"].as_str() {
            Some(cwd) if Path::new(cwd).is_dir() => PathBuf::from(cwd),
            // One component at a time: a `/` kept inside the path on Windows made the recorded folder differ from the
            // one Vibe reports (`...\\seed\\work/alpha`).
            Some(cwd) => cwd
                .split('/')
                .filter(|part| !part.is_empty())
                .fold(data_dir.join("seed"), |path, part| path.join(part)),
            None => {
                // A local Vibe always gives a chat a folder (its own when none is named).
                unsupported.push(json!({"id": seed_id, "why": "a chat without a folder: the local Vibe always has one"}));
                continue;
            }
        };
        std::fs::create_dir_all(&folder).unwrap_or_else(|e| fail(&format!("seed: {e}")));
        if let Err(e) = core.register_picked_folder(&folder) {
            fail(&format!("seed: {e:?}"));
        }
        let sid = match run(
            core,
            json!({"type": "startSession", "agentConfig": {"cwd": folder, "trustWorkspace": true}}),
        )
        .await
        {
            Ok(CommandResult::Session { snapshot, .. }) => snapshot.session_id,
            other => fail(&format!("seed {seed_id}: {other:?}")),
        };
        let mut turns: Vec<String> = s["history"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["role"] == "user")
            .map(|m| super::scripted_model::text_of(&m["content"]))
            .collect();
        if turns.is_empty() {
            turns.push(
                s["preview"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .unwrap_or("seeded")
                    .to_owned(),
            );
        }
        for (index, text) in turns.into_iter().enumerate() {
            if let Err(e) = run(
                core,
                json!({"type": "startTurn", "sessionId": sid, "message": [{"type": "text", "text": text}]}),
            )
            .await
            {
                fail(&format!("seed {seed_id}: {e}"));
            }
            if let Err(e) = quiet(core, &sid, index + 1).await {
                fail(&e);
            }
        }
        let mut steps = Vec::new();
        if s["isUnseen"] != true {
            steps.push(("session/markAsSeen", json!({"sessionId":sid})));
        }
        if let Some(title) = s["title"].as_str() {
            steps.push(("session/rename", json!({"sessionId": sid, "title": title})));
        }
        if s["pinnedAt"].is_number() {
            steps.push(("session/pin", json!({"sessionId": sid, "pinned": true})));
        }
        if s["archivedAt"].is_number() {
            steps.push((
                "session/archive",
                json!({"sessionId": sid, "archived": true}),
            ));
        }
        for (method, params) in steps {
            if let Err(e) = call(core, method, params).await {
                fail(&format!("seed {seed_id}: {method}: {e}"));
            }
        }
        if let Some(agent) = s["agent"]["name"]
            .as_str()
            .filter(|agent| *agent != "custom")
        {
            call(
                core,
                "session/agent/update",
                json!({"sessionId":sid,"agentName":agent}),
            )
            .await
            .unwrap();
        }
        let _ = run(core, json!({"type": "stopSession", "sessionId": sid})).await;
        folders.insert(seed_id.clone(), json!(folder));
        ids.insert(seed_id, json!(sid));
    }
    for s in sessions.iter().filter(|s| s["parentSessionId"].is_string()) {
        let seed_id = s["id"].as_str().unwrap();
        let parent_seed = s["parentSessionId"].as_str().unwrap();
        let parent = ids[parent_seed]
            .as_str()
            .unwrap_or_else(|| fail("seed child has no real parent"));
        let state = match run(core, json!({"type":"readSession","sessionId":parent})).await {
            Ok(CommandResult::Session { snapshot, .. }) => snapshot.state,
            other => fail(&format!("seed parent: {other:?}")),
        };
        let minimum = state["turns"].as_array().map_or(0, Vec::len) + 1;
        run(core, json!({"type":"startTurn","sessionId":parent,"message":[{"type":"text","text":format!("rv1-create-child:{seed_id}")}]})).await
            .unwrap_or_else(|e| fail(&format!("seed child: {e}")));
        quiet(core, parent, minimum)
            .await
            .unwrap_or_else(|e| fail(&e));
        let children = match run(core, json!({"type":"call","method":"session/list","params":{"parentSessionId":parent,"limit":500}})).await {
            Ok(CommandResult::Reply {result,..}) => result,
            other => fail(&format!("seed children: {other:?}")),
        };
        let children = children["items"].as_array().unwrap();
        let child = children
            .iter()
            .find(|child| !ids.values().any(|id| id == &child["id"]))
            .unwrap_or_else(|| fail(&format!("the real subagent made no child: {children:?}")));
        let sid = child["id"].as_str().unwrap();
        assert_eq!(child["parentSessionId"], parent);
        let folder = folders[parent_seed].clone();
        let _ = run(core, json!({"type":"stopSession","sessionId":parent})).await;
        folders.insert(seed_id.to_owned(), folder);
        ids.insert(seed_id.to_owned(), json!(sid));
    }
    model.clear_hook();
    let record = json!({"ids": ids, "folders": folders, "unsupported": unsupported});
    std::fs::write(data_dir.join("seed-ids.json"), record.to_string())
        .unwrap_or_else(|e| fail(&format!("seed: {e}")));
    eprintln!(
        "shell_host: seeded {} real chats in {:?} ({} unsupported)",
        ids_len(&record),
        started.elapsed(),
        unsupported_len(&record)
    );
}

fn ids_len(record: &Value) -> usize {
    record["ids"].as_object().map_or(0, serde_json::Map::len)
}

fn unsupported_len(record: &Value) -> usize {
    record["unsupported"].as_array().map_or(0, Vec::len)
}
