//! A core over the multiplexed scripted fake (`fake_vibe_server --multiplex --script`) with seeded
//! stored sessions `old-1`..`old-4` and the fake's per-request channel log (the fixture). Used by
//! `vw_core_channels.rs` and `vw_core_restore.rs`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vw_core_api::{
    AlertKind, Command, CommandResult, CoreConfig, CoreEvent, CoreStatus, CoreSubscription, Origin,
    SessionChange,
};

use super::core::{ScriptRig, script_core_with};
use super::fake::scratch;

pub struct Rig {
    pub rig: ScriptRig,
    pub channels: PathBuf,
}

/// The seeded stored sessions `old-1`..`old-4` (no recorded profile).
pub fn default_seed() -> Vec<Value> {
    (1..=4)
        .map(|n| {
            json!({"id": format!("old-{n}"), "title": format!("Old {n}"), "status": {"type": "idle"},
                   "createdAt": n, "updatedAt": 100 - n, "cwd": "/work/alpha"})
        })
        .collect()
}

/// A core over the multiplexed fake with seeded stored sessions `old-1`..`old-4` and status events.
pub async fn rig(name: &str, extra: &[&str], tweak: impl FnOnce(&mut CoreConfig)) -> Rig {
    let side = scratch(&format!("{name}-side"));
    rig_in(name, &side, &default_seed(), extra, tweak).await
}

/// [`rig`] with the side files (seed, channel log, `--persist` file) in `side`, which is kept as is
/// (two cores in a row can share one persisted fake world), and the given seed.
pub async fn rig_in(
    name: &str,
    side: &std::path::Path,
    items: &[Value],
    extra: &[&str],
    tweak: impl FnOnce(&mut CoreConfig),
) -> Rig {
    let seed = side.join("seed.json");
    std::fs::write(&seed, serde_json::to_vec(items).unwrap()).unwrap();
    let channels = side.join("channels.log");
    let (seed, log) = (
        seed.to_string_lossy().into_owned(),
        channels.to_string_lossy().into_owned(),
    );
    let mut args = vec![
        "--status-events",
        "--seed",
        seed.as_str(),
        "--log-channels",
        log.as_str(),
    ];
    args.extend_from_slice(extra);
    Rig {
        rig: script_core_with(name, &args, tweak).await,
        channels,
    }
}

impl Rig {
    /// `(channel, method, session)` of every request the fake received, `initialize` included.
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
            .run(Command::StartSession { agent_config: None })
            .await
            .unwrap()
        {
            CommandResult::Session { snapshot, .. } => snapshot.session_id,
            other => panic!("{other:?}"),
        }
    }

    pub async fn begin_turn(&self, sid: &str) {
        self.run(Command::StartTurn {
            session_id: sid.to_owned(),
            message: vec![json!({"type": "text", "text": "go"})],
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
    matches!(e, CoreEvent::Change { change: SessionChange::SessionPatched { patch, .. } }
        if patch.iter().any(|op| op["path"] == "/status" && op["value"]["type"] == "idle"))
}

/// Polls `cond` every 20 ms for up to six times `limit` (the fixture: see `fake::eventually`).
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
    let mut sub = r.rig.core.subscribe(Some(sid.to_owned()));
    r.begin_turn(sid).await;
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
            Ok(Some(event)) if went_idle(&event) => return false,
            _ => {}
        }
    }
    panic!("the turn of {sid} neither asked nor finished");
}

pub fn alerted(kind: AlertKind, sid: &str) -> impl Fn(&CoreEvent) -> bool + '_ {
    move |e| matches!(e, CoreEvent::Alert { kind: k, session_id: Some(s), .. } if *k == kind && s == sid)
}
