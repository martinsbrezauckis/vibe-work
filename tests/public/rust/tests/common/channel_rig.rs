//! Real pinned Vibe with durable fixture sessions and passive per-channel request logs.
//! Used by channel, restart and setting tests; model tool requests exercise real approval callbacks.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vw_core_api::{
    AlertKind, Command, CommandResult, CoreConfig, CoreEvent, CoreStatus, CoreSubscription, Origin,
    SessionChange,
};

use super::core::{ScriptRig, stored_core_with_server};
use super::processes::scratch;

pub struct Rig {
    pub rig: ScriptRig,
    pub channels: PathBuf,
}

/// The seeded stored sessions `old-1`..`old-4` (no recorded profile).
/// Four stored chats in Ask. Since Vibe 3.0.1 every turn records the session's agent, and a reopened chat runs in
/// that record; the seeds pin Ask so their turns keep asking (the seed default would be `accept-edits`).
pub fn default_seed() -> Vec<Value> {
    (1..=4)
        .map(|n| {
            json!({"id": format!("old-{n}"), "title": format!("Old {n}"), "status": {"type": "idle"},
                   "createdAt": n, "updatedAt": 100 - n, "cwd": "/work/alpha", "agent": {"name": "ask"}})
        })
        .collect()
}

/// A core over real Vibe with seeded stored sessions `old-1`..`old-4` and status events.
pub async fn rig(name: &str, extra: &[&str], tweak: impl FnOnce(&mut CoreConfig)) -> Rig {
    let side = scratch(&format!("{name}-side"));
    rig_in(name, &side, &default_seed(), extra, tweak).await
}

/// [`rig`] with the real Vibe home in `side`, which is kept as is
/// (two cores in a row can share one stored real sessions), and the given seed.
pub async fn rig_in(
    name: &str,
    side: &std::path::Path,
    items: &[Value],
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> Rig {
    rig_in_fault(name, side, items, extra, tweak, Value::Null).await
}

pub async fn rig_in_fault(
    name: &str,
    side: &std::path::Path,
    items: &[Value],
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
    mut fault: Value,
) -> Rig {
    assert!(
        extra.is_empty(),
        "{name}: obsolete server options: {extra:?}"
    );
    let rig = stored_core_with_server(
        name,
        Some(&side.join("vibe-home")),
        items,
        {
            let mut script = super::shared_real_vibe::approval_script();
            script.markers = true;
            script
        },
        |config| {
            tweak(config);
            // Fixture aliases in local app input become the IDs captured from real Vibe, before Core loads them.
            if let Some(dir) = &config.sessions_dir {
                let file = dir.join("access.json");
                if file.is_file() {
                    let captures: Value = serde_json::from_slice(
                        &std::fs::read(side.join("vibe-home/fixture-session-ids.json")).unwrap(),
                    )
                    .unwrap();
                    let input: serde_json::Map<String, Value> =
                        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                    let actual: serde_json::Map<String, Value> = input
                        .into_iter()
                        .map(|(alias, value)| {
                            (
                                captures["ids"][&alias]
                                    .as_str()
                                    .unwrap_or(&alias)
                                    .to_owned(),
                                value,
                            )
                        })
                        .collect();
                    std::fs::write(file, serde_json::to_vec(&actual).unwrap()).unwrap();
                }
            }
        },
        move |server| {
            if !fault.is_null() {
                if let Some(rules) = fault["onRequest"].as_array_mut() {
                    let captures: Value = serde_json::from_slice(
                        &std::fs::read(server.vibe_home.join("fixture-session-ids.json")).unwrap(),
                    )
                    .unwrap();
                    for rule in rules {
                        if let Some(alias) = rule["sessionAlias"].as_str() {
                            let id = captures["ids"][alias].clone();
                            rule["sessionId"] = id;
                        }
                    }
                }

                let dir = server.cwd.clone();
                super::process_fault::install(server, &dir, fault);
            }
        },
    )
    .await;
    let channels = rig.dir.join("channels.log");
    Rig { rig, channels }
}

impl Rig {
    pub fn id(&self, alias: &str) -> String {
        self.rig.id(alias)
    }

    /// `(channel, method, session)` of every request real Vibe received, `initialize` included.
    pub fn log(&self) -> Vec<(u64, String, String)> {
        std::fs::read_to_string(&self.channels)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let mut parts = line.split(' ');
                let channel = parts.next().unwrap().parse().unwrap();
                let method = parts.next().unwrap().to_owned();
                let session = parts.next().unwrap_or("-").to_owned();
                (channel, method, session)
            })
            .collect()
    }

    /// Channels on which `method` for `session` arrived, in order.
    pub fn on(&self, method: &str, session: &str) -> Vec<u64> {
        self.log()
            .into_iter()
            .filter(|(_, m, s)| m == method && s == session)
            .map(|(c, _, _)| c)
            .collect()
    }

    pub async fn run(&self, command: Command) -> Result<CommandResult, vw_core_api::CoreError> {
        self.rig.core.execute(Origin::Ui, command).await
    }

    pub fn status(&self) -> CoreStatus {
        self.rig.core.status()
    }

    pub fn channel_of(&self, session: &str) -> Option<u64> {
        self.status()
            .channels
            .open
            .iter()
            .find(|c| c.session_id == session)
            .map(|c| c.channel_id)
    }

    pub async fn start(&self) -> String {
        match self
            .run(Command::StartSession {
                agent_config: Some(json!({"agent":"ask"})),
            })
            .await
            .unwrap()
        {
            CommandResult::Session { snapshot, .. } => snapshot.session_id,
            other => panic!("{other:?}"),
        }
    }

    pub async fn begin_turn(&self, sid: &str) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.run(Command::StartTurn {
            session_id: sid.to_owned(),
            message: vec![json!({"type": "text", "text": format!("go {sid}-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))})],
            client_user_message_id: None,
        })
        .await
        .unwrap();
    }

    pub async fn answer(&self, sid: &str, callback: String) {
        self.run(Command::AnswerCallback {
            session_id: sid.to_owned(),
            callback_id: callback,
            output: json!({"type": "approval", "decision": {"type": "approve"}}),
        })
        .await
        .unwrap();
    }

    /// One whole turn (approval answered at once); returns its duration.
    pub async fn full_turn(&self, sid: &str) -> Duration {
        let mut sub = self.rig.core.subscribe(Some(sid.to_owned()));
        let started = Instant::now();
        self.begin_turn(sid).await;
        let callback = opened(&mut sub, sid).await;
        self.answer(sid, callback).await;
        until(&mut sub, went_idle).await;
        started.elapsed()
    }

    pub fn call(method: &str, params: Value) -> Command {
        Command::Call {
            method: method.to_owned(),
            params,
            confirmation_token: None,
        }
    }
}

pub async fn until(sub: &mut CoreSubscription, want: impl Fn(&CoreEvent) -> bool) {
    let limit = Instant::now() + Duration::from_secs(60);
    while Instant::now() < limit {
        if let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(200), sub.next()).await
            && want(&event)
        {
            return;
        }
    }
    panic!("event did not arrive");
}

pub async fn opened(sub: &mut CoreSubscription, sid: &str) -> String {
    let limit = Instant::now() + Duration::from_secs(60);
    while Instant::now() < limit {
        if let Ok(Some(CoreEvent::CallbackOpened {
            session_id,
            callback_id,
            ..
        })) = tokio::time::timeout(Duration::from_millis(200), sub.next()).await
            && session_id == sid
        {
            return callback_id;
        }
    }
    panic!("no callback opened for {sid}");
}

pub fn went_idle(e: &CoreEvent) -> bool {
    match e {
        CoreEvent::Change {
            change: SessionChange::SessionPatched { patch, .. },
        } => patch.iter().any(|op| {
            (op["path"] == "/status" && op["value"]["type"] == "idle")
                || (op["path"] == "/status/type" && op["value"] == "idle")
        }),
        CoreEvent::Snapshot { snapshot } => {
            snapshot.state["session"]["status"]["type"] == "idle"
                && snapshot.state["turns"]
                    .as_array()
                    .is_some_and(|turns| turns.last().is_some_and(|t| t["status"] == "completed"))
        }
        _ => false,
    }
}

/// Polls `cond` every 20 ms for up to six times `limit` (the fixture: see `processes::eventually`).
pub async fn eventually(limit: Duration, cond: impl Fn() -> bool) -> bool {
    let limit = limit * 6;
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}

pub async fn active_agent(r: &Rig, sid: &str) -> String {
    let CommandResult::Reply { result, .. } = r
        .run(Rig::call("runtime/read", json!({"sessionId": sid})))
        .await
        .unwrap()
    else {
        panic!()
    };
    result["runtime"]["activeAgent"]["name"]
        .as_str()
        .unwrap_or("?")
        .to_owned()
}

/// Starts a turn on `sid` and reports whether it asked for an approval (`true`, then answered) or
/// finished without asking (`false`).
pub async fn asks(r: &Rig, sid: &str) -> bool {
    let previous = match r
        .run(Command::RefreshSession {
            session_id: sid.to_owned(),
        })
        .await
    {
        Ok(CommandResult::Session { snapshot, .. }) => {
            snapshot.state["turns"].as_array().map_or(0, Vec::len)
        }
        _ => 0,
    };
    let mut sub = r.rig.core.subscribe(Some(sid.to_owned()));
    r.run(Command::StartTurn {
        session_id: sid.to_owned(),
        message: vec![json!({"type":"text","text":"vw-bash: python -c 'print(12345)'"})],
        client_user_message_id: None,
    })
    .await
    .unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    while Instant::now() < end {
        match tokio::time::timeout(Duration::from_millis(200), sub.next()).await {
            Ok(Some(CoreEvent::CallbackOpened {
                session_id,
                callback_id,
                ..
            })) if session_id == sid => {
                r.answer(sid, callback_id).await;
                until(&mut sub, went_idle).await;
                return true;
            }
            Ok(Some(event)) if went_idle(&event) => {
                if let Ok(CommandResult::Session { snapshot, .. }) = r
                    .run(Command::ReadSession {
                        session_id: sid.to_owned(),
                    })
                    .await
                    && snapshot.state["turns"].as_array().is_some_and(|turns| {
                        turns.len() > previous
                            && turns
                                .last()
                                .is_some_and(|turn| turn["status"] == "completed")
                    })
                {
                    return false;
                }
            }
            _ => {}
        }
    }
    panic!("the turn of {sid} neither asked nor finished");
}

pub fn alerted(kind: AlertKind, sid: &str) -> impl Fn(&CoreEvent) -> bool + '_ {
    move |e| matches!(e, CoreEvent::Alert { kind: k, session_id: Some(s), .. } if *k == kind && s == sid)
}
