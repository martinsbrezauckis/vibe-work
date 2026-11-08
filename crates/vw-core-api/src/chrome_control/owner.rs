//! App-lifetime owner of the helper's private pipes. Depends on tokio and the key service;
//! called by core startup, Settings and the checked gateway. Requests are never replayed after pipe loss.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::core::Shared;

use super::{PairingState, mapping, setup};

#[path = "owner_pairing.rs"]
mod pairing;

pub(super) const TOOLS: [&str; 3] = ["chrome_status", "chrome_session", "chrome_execute"];
const UNKNOWN: &str = vw_vocabulary::texts::CHROME_CONNECTION_ENDED;

struct Call {
    name: String,
    args: Value,
    reply: oneshot::Sender<Answer>,
    /// A plain owner request (`name` is the method, e.g. `owner.enroll.mint`) rather than a `tools/call`; its
    /// JSON-RPC error answer is an outcome, not a broken connection (`Pipe::begin_tolerant`).
    rpc: bool,
}

/// An owner answer and who wrote it. The helper's answers can carry page-derived text (titles, URLs, page text and
/// errors, console lines, dialog text, download names), so they reach the model inside the untrusted-content envelope
/// (`content::for_model`). The core's own failures (not running, busy, restarting, connection lost) hold only Vibe
/// Work's fixed words and reach the model as the tool's own error, outside it.
pub(super) enum Answer {
    Helper(Value),
    Own(Value),
}

impl Answer {
    pub(super) fn value(&self) -> &Value {
        match self {
            Self::Helper(value) | Self::Own(value) => value,
        }
    }

    pub(super) fn value_mut(&mut self) -> &mut Value {
        match self {
            Self::Helper(value) | Self::Own(value) => value,
        }
    }

    pub(super) fn into_value(self) -> Value {
        match self {
            Self::Helper(value) | Self::Own(value) => value,
        }
    }
}

/// The core's own failure as an answer (never wrapped as page content).
pub(super) fn own(text: &str, code: &str) -> Answer {
    Answer::Own(failure(text, code))
}

struct Running {
    send: mpsc::Sender<Call>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

type SetupSnapshot = (setup::ChromeSetupView, Option<setup::ChromeSetupView>);

#[derive(Default)]
pub(super) struct Owner {
    pub(super) approvals: super::pairing_approval::Approvals,
    running: Mutex<Option<Running>>,
    pub(super) tools: Mutex<Vec<Value>>,
    pub(super) pairing: Mutex<(PairingState, Option<String>)>,
    pub(super) safari_pairing: Mutex<(String, Option<String>)>,
    /// Safari pairing states received from the helper; a change means an extension port is open.
    pub(super) safari_seen: std::sync::atomic::AtomicU64,
    pub(super) safari_error: Mutex<Option<super::safari::ConnectionError>>,
    pub(super) firefox_pairing: Mutex<(PairingState, Option<String>)>,
    pub(super) firefox_states: Mutex<Vec<String>>,
    pub(super) edge_pairing: Mutex<(PairingState, Option<String>)>,
    observed_setup: Mutex<Option<SetupSnapshot>>,
    pub(super) credential_missing: std::sync::atomic::AtomicBool,
    pub(super) connected_profiles: super::addon::ConnectedProfiles,
    pub(super) profile_ids: super::profile_ids::ProfileIds,
    pub(super) wake: super::reconnect::Wake,
    pub(super) starts: super::browser_start::Attempts,
    pub(super) connection_waiting: super::connection_notice::Waiting,
    /// The automatic-pairing attempt and what the current helper connection reported for it (`enroll`).
    pub(super) enroll: Mutex<super::enroll::Enroll>,
    /// Edge's waiting state and attempt never consume Chrome's Connect (or conversely).
    pub(super) edge_enroll: Mutex<super::enroll::Enroll>,
}

impl Owner {
    pub(super) fn control(&self, mut args: Value) {
        self.profile_ids.normalize(&mut args);
        let send = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|run| run.send.clone());
        if let Some(send) = send {
            let (reply, _answer) = oneshot::channel();
            let call = Call {
                name: "chrome_session".into(),
                args,
                reply,
                rpc: false,
            };
            if let Err(tokio::sync::mpsc::error::TrySendError::Full(call)) = send.try_send(call) {
                tokio::spawn(async move {
                    let _ = send.send(call).await;
                });
            }
        }
    }

    /// Tests: the owner as if running, answering each call with `answer(name, args)`; returns every call it saw.
    #[cfg(test)]
    pub(super) fn answering(
        &self,
        answer: impl Fn(&str, &Value) -> Value + Send + 'static,
    ) -> std::sync::Arc<Mutex<Vec<(String, Value)>>> {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let (send, mut calls) = mpsc::channel::<Call>(16);
        let (stop, mut stopped) = watch::channel(false);
        let log = seen.clone();
        // Ends on `stop()` like the real owner task (which holds the sender, so the channel never closes first).
        let task = tokio::spawn(async move {
            loop {
                let call = tokio::select! {
                    call = calls.recv() => call,
                    _ = stopped.changed() => None,
                };
                let Some(call) = call else { break };
                let value = answer(&call.name, &call.args);
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((call.name, call.args));
                let _ = call.reply.send(Answer::Helper(value));
            }
        });
        *self.running.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Running { send, stop, task });
        seen
    }

    pub(super) async fn stop(&self) {
        self.approvals.clear();
        // Turning Use my Chrome off, removing and quitting drop every ticket: the helper also loses them when it
        // ends, the revoke makes that explicit while it still runs (bounded; an older helper ignores it).
        let knows = [&self.enroll, &self.edge_enroll].iter().any(|enroll| {
            enroll
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .helper_knows_enroll()
        });
        if knows {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                self.rpc(super::enroll::REVOKE, json!({})),
            )
            .await;
        }
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(running) = running {
            let _ = running.stop.send(true);
            let _ = running.task.await;
        }
        self.clear_connection();
    }

    /// Whether the owner task runs (a helper connection may still be coming).
    pub(super) fn is_running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn clear_connection(&self) {
        self.approvals.clear();
        *self
            .safari_pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (String::new(), None);
        self.wake.clear();
        self.starts.clear();
        self.profile_ids.clear();
        self.connected_profiles.clear();
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        *self.pairing.lock().unwrap_or_else(PoisonError::into_inner) =
            (PairingState::Disconnected, None);
        *self
            .firefox_pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (PairingState::Unknown, None);
        self.firefox_states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        *self
            .edge_pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (PairingState::Unknown, None);
        // A helper that ended took its tickets with it; the next connection reports its own waiting state.
        self.enroll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connection_ended(super::enroll_browser::Browser::Chrome);
        self.edge_enroll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connection_ended(super::enroll_browser::Browser::Edge);
    }

    /// One owner request on the private pipe (`method` is not a tool). `None` when the helper is not connected,
    /// the request timed out, the connection ended, or the helper answered a JSON-RPC error (an older helper).
    pub(super) async fn rpc(&self, method: &str, params: Value) -> Option<Value> {
        let send = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|run| run.send.clone())?;
        match Self::request(send, method, params, true).await? {
            Answer::Helper(value) if value.get("rpcError").is_none() => Some(value),
            _ => None,
        }
    }

    pub(super) async fn call(&self, name: &str, mut args: Value) -> Answer {
        if self.profile_ids.needs_status(&args) {
            let status = self.call_raw("chrome_status", json!({})).await;
            self.profile_ids
                .observe(&status.value()["structuredContent"]);
        }
        self.profile_ids.normalize(&mut args);
        self.call_raw(name, args).await
    }

    async fn call_raw(&self, name: &str, args: Value) -> Answer {
        let send = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|run| run.send.clone());
        let Some(send) = send else {
            return own(vw_vocabulary::texts::CHROME_NOT_RUNNING, "OWNER_REQUIRED");
        };
        Self::request(send, name, args, false)
            .await
            .unwrap_or_else(|| own(UNKNOWN, "TRANSPORT_LOST"))
    }

    /// Bound queue admission and the answer together: startup may never reach the request dispatcher.
    async fn request(
        send: mpsc::Sender<Call>,
        name: &str,
        args: Value,
        rpc: bool,
    ) -> Option<Answer> {
        tokio::time::timeout(Duration::from_secs(35), async {
            let (reply, answer) = oneshot::channel();
            send.send(Call {
                name: name.to_owned(),
                args,
                reply,
                rpc,
            })
            .await
            .ok()?;
            answer.await.ok()
        })
        .await
        .ok()
        .flatten()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(run) = self
            .running
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // Dropping the sender wakes the task, closes stdin and reaps the child.
            let _ = run.stop.send(true);
        }
    }
}

pub(super) fn failure(text: &str, code: &str) -> Value {
    json!({"isError":false,"content":[{"type":"text","text":text}],"structuredContent":{"error":{"code":code,"message":text}}})
}

/// Uses the same default endpoint as Chrome's native host. Scratch homes select isolated disposable endpoints.
pub(super) fn state_dir(config: &setup::ChromeControlConfig) -> Option<PathBuf> {
    if !config.available() {
        return None;
    }
    if let Some(home) = &config.user_home {
        return Some(home.join("VibeWork").join("ChromeControl"));
    }
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("VibeWork").join("ChromeControl"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/VibeWork/ChromeControl"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
            .map(|p| p.join("vibework/chrome-control"))
    }
    .filter(|p| p.is_absolute())
}

pub(crate) async fn start(shared: &Arc<Shared>) -> Result<(), String> {
    let chrome = &shared.chrome_control;
    if !chrome.any_enabled() && !super::safari::enabled(shared) {
        return Ok(());
    }
    let Some(dir) = chrome.dir() else {
        return Ok(());
    };
    if setup::view(&chrome.config, Some(dir), &shared.instructions.own_home).state
        == setup::SetupState::NotSetUp
        && !(cfg!(feature = "your-firefox") && dir.join("firefox-window").is_file())
        && !super::safari::enabled(shared)
    {
        return Ok(());
    }
    if chrome
        .owner
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
    {
        return Ok(());
    }
    let keys = shared
        .config
        .keys
        .as_ref()
        .ok_or(vw_vocabulary::texts::CHROME_NEEDS_KEYRING)?;
    let secret = keys
        .chrome_owner()
        .await
        .map_err(|_| vw_vocabulary::texts::CHROME_OWNER_READ_FAILED)?;
    chrome
        .owner
        .credential_missing
        .store(secret.is_none(), std::sync::atomic::Ordering::SeqCst);
    if secret.is_none() {
        publish(shared);
    }
    let secret = secret.ok_or(vw_vocabulary::texts::CHROME_OWNER_MISSING)?;

    let (_, bytes) = setup::packaged_helper(&chrome.config)?;
    let expected = blake3::hash(&bytes);
    let package = chrome.config.clone();
    let (send, requests) = mpsc::channel(32);
    let (stop, stopped) = watch::channel(false);
    let weak = Arc::downgrade(shared);
    let dir = dir.to_owned();
    let task = tokio::spawn(supervise(
        weak, package, dir, expected, secret, requests, stopped,
    ));
    let mut running = chrome
        .owner
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if running.is_some() {
        let _ = stop.send(true);
    } else {
        *running = Some(Running { send, stop, task });
    }
    Ok(())
}

pub(super) fn publish(shared: &Shared) {
    shared.emit(crate::CoreEvent::ChromeControl {
        view: super::commands::view(shared),
    });
}

pub(super) fn observe(shared: &Weak<Shared>, message: &Value) -> Result<(), ()> {
    let Some(shared) = shared.upgrade() else {
        return Err(());
    };
    if message["method"] == "notifications/owner/safari"
        && let Some(state) = message["params"]["state"].as_str()
    {
        super::safari::connection(&shared, state);
    }
    if message["method"] == "notifications/owner/pairingApproval" {
        super::pairing_approval::observe(&shared, &message["params"]);
    }
    if message["method"] == "notifications/client/selection" {
        let changed = shared
            .chrome_control
            .selections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .receive(&message["params"], |id| shared.store.view(id).is_some());
        if changed {
            publish(&shared);
        }
    }
    if message["method"] == "notifications/owner/pairing"
        && let Some(result) = pairing_result(&message["params"])
    {
        super::safari::observe(&shared, &message["params"]);
        pairing::observe(&shared, &message["params"], result);
    }
    Ok(())
}

/// The helper's signed pairing state (protocol v6 `notifications/owner/pairing`) in the shape [`update_pairing`]
/// already reads. `waiting` / `paired` carry this Vibe Work's own fingerprint, shown so the person can compare; for
/// `mismatch` the fingerprint is the other identity Chrome holds, so it never replaces ours on screen. An unknown
/// state is ignored (the helper validates and bounds these events; this only informs, it admits nothing).
fn pairing_result(params: &Value) -> Option<Value> {
    let fingerprint = params["fingerprint"].clone();
    match params["state"].as_str()? {
        "waiting" => Some(json!({"structuredContent":{
            "error":{"code":"OWNER_APPROVAL_REQUIRED"},"ownerFingerprint":fingerprint}})),
        "paired" => Some(json!({"structuredContent":{
            "error":{"code":"OWNER_CONNECTING"},"ownerFingerprint":fingerprint}})),
        "mismatch" => {
            Some(json!({"structuredContent":{"error":{"code":"OWNER_IDENTITY_MISMATCH"}}}))
        }
        _ => None,
    }
}

pub(super) fn update_pairing(shared: &Weak<Shared>, result: &Value) {
    update_pairing_state(shared, result);
    if let Some(shared) = shared.upgrade() {
        shared.chrome_control.owner.starts.connected(&shared);
        super::connection_notice::connected(&shared);
    }
}

fn update_pairing_state(shared: &Weak<Shared>, result: &Value) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let mut data = result["structuredContent"].clone();
    shared.chrome_control.owner.wake.observe(&data);
    shared.chrome_control.owner.profile_ids.observe(&data);
    // Keep all browsers' versions before the Chrome-only pairing view below filters them.
    if data["profiles"].is_array() && data.get("profileId").is_none() {
        shared
            .chrome_control
            .owner
            .connected_profiles
            .observe(&data);
    }
    if cfg!(any(feature = "your-edge", feature = "your-firefox")) {
        super::edge_state::update(&shared, &data);
    }
    if data["profileId"]
        .as_str()
        .is_some_and(|id| !super::edge_state::chrome_profile(id))
    {
        return;
    }
    if let Some(profiles) = data.get_mut("profiles").and_then(Value::as_array_mut) {
        profiles.retain(|profile| {
            let id = profile["profileId"].as_str();
            if cfg!(any(feature = "your-edge", feature = "your-firefox")) {
                id.is_some_and(super::edge_state::chrome_profile)
            } else {
                // Chrome-only helpers historically used unscoped labels; foreign scopes remain foreign.
                id.is_none_or(|id| !id.contains(':') || id.starts_with("chrome:"))
            }
        });
    }
    let data = &data;
    let code = data["error"]["code"].as_str().unwrap_or_default();
    let connected = data["profiles"]
        .as_array()
        .is_some_and(|p| p.iter().any(|p| p["connected"] == true));
    // Only a status answer (its profiles) or the helper's own pairing/owner report says what is connected. An
    // action's failure (a closed tab, an unknown profile, an older extension) says nothing about the connection:
    // taking it as "Not connected" flipped the card for two seconds and replaced the action's own error.
    if data.get("profiles").is_none() && !code.starts_with("OWNER_") {
        return;
    }
    // Every helper status answer names our own fingerprint, with or without a waiting extension: only the helper's
    // waiting report is the pairing step (`addon`), else nothing is connected.
    let waiting = shared
        .chrome_control
        .owner
        .enroll
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .waiting_reported();
    let state = PairingState::observe(code, connected, waiting);
    // A status poll has no registered profile while an enabled extension holds another identity.
    // Keep its signed mismatch until a pairing report, live connection or helper reset replaces it.
    if !connected
        && code.is_empty()
        && shared
            .chrome_control
            .owner
            .pairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            == PairingState::OtherOwner
    {
        return;
    }
    if connected {
        shared.chrome_control.mark_paired();
        shared
            .chrome_control
            .owner
            .enroll
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .connected();
    }
    let mut current = shared
        .chrome_control
        .owner
        .pairing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let fingerprint = data["ownerFingerprint"]
        .as_str()
        .map(|fingerprint| fingerprint.chars().take(16).collect())
        .or_else(|| current.1.clone());
    let next = (state, fingerprint);
    let changed = *current != next;
    *current = next;
    drop(current);
    publish_setup_if_changed(&shared, changed);
}

/// A regular status poll also discovers either browser's extension directory appearing after pairing.
pub(super) fn publish_setup_if_changed(shared: &Shared, changed: bool) {
    let view = super::commands::view(shared);
    let next = (view.setup.clone(), view.edge.as_deref().cloned());
    let mut previous = shared
        .chrome_control
        .owner
        .observed_setup
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if changed || previous.as_ref() != Some(&next) {
        *previous = Some(next);
        drop(previous);
        shared.emit(crate::CoreEvent::ChromeControl { view });
    }
}

#[cfg(test)]
pub(super) fn update_work_for_test(shared: &Arc<Shared>, result: &Value) {
    update_work(&Arc::downgrade(shared), result, tokio::time::Instant::now());
}

pub(super) fn update_work(shared: &Weak<Shared>, result: &Value, started: tokio::time::Instant) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let Some(rows) = result["structuredContent"]["sessions"].as_array() else {
        return;
    };
    let mut tasks = shared
        .chrome_control
        .tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut work = shared
        .chrome_control
        .work
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // A live session the helper reports stopped without Vibe Work asking: the person's Stop in Chrome.
    let stopped = work.stopped_unasked(rows, started);
    let changed = work.observe(rows, &mut tasks, started);
    drop(work);
    drop(tasks);
    super::person_stop::received(&shared, &stopped);
    if changed {
        publish(&shared);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one connection loop: launch, synchronize, serve calls and polls, then report why it ended"
)]
async fn supervise(
    shared: Weak<Shared>,
    package: setup::ChromeControlConfig,
    dir: PathBuf,
    expected: blake3::Hash,
    secret: vw_vault::SecretString,
    mut requests: mpsc::Receiver<Call>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = 100;
    while !*stop.borrow() {
        let result = tokio::select! {
            _ = stop.changed() => return,
            result = super::pipe::Pipe::launch(&package, &dir, expected, &secret, shared.clone()) => result,
        };
        if result.is_err() {
            tracing::debug!("Chrome Control helper did not start or was refused");
        }
        if let Ok(mut pipe) = result {
            if let Some(shared) = shared.upgrade() {
                shared
                    .chrome_control
                    .tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            }
            'connection: {
                let mut sent_policy = Value::Null;
                let synchronized = tokio::select! {
                    _ = stop.changed() => false,
                    result = synchronize(&shared, &mut pipe, &mut sent_policy) => result.is_ok(),
                };
                if !synchronized {
                    // Failed startup needs the same disconnect, queue rejection and backoff as a lost pipe.
                    break 'connection;
                }
                // A new helper connection has acknowledged no switch value yet: send the current one (retried
                // until acknowledged).
                if let Some(core) = shared.upgrade() {
                    core.chrome_control.forget_local_network_ack();
                    tokio::spawn(async move { super::send_local_network(&core).await });
                }
                let mut pending = std::collections::HashMap::<
                    u64,
                    (Option<oneshot::Sender<Answer>>, tokio::time::Instant),
                >::new();
                let mut poll = tokio::time::interval(Duration::from_secs(2));
                let mut expiry = tokio::time::interval(Duration::from_millis(100));
                // Diagnostics: why this connection ended (a fixed word; no helper or page text).
                let mut ended = "stopped";
                loop {
                    tokio::select! {
                        biased;
                        _ = stop.changed() => { break; }
                        reply = pipe.next() => {
                            let Ok((id, result)) = reply else { ended = "helper pipe closed"; break };
                            let Some((reply, started)) = pending.remove(&id) else { ended = "unexpected reply"; break };
                            update_pairing(&shared, &result);
                            update_work(&shared, &result, started);
                            if let Some(reply) = reply { let _ = reply.send(Answer::Helper(result)); }
                        }
                        call = requests.recv() => {
                            let Some(call) = call else { ended = "requests closed"; break };
                            if call.reply.is_closed() && !(call.name == "chrome_session" && matches!(call.args["action"].as_str(), Some("stop" | "delete" | "cancel"))) { continue; }
                            if pending.len() >= 32 && call.name != "chrome_session" {
                                let _ = call.reply.send(own("Not run: Chrome is busy.", "BUSY"));
                                continue;
                            }
                            let begun = if call.rpc {
                                pipe.begin_tolerant(&call.name, call.args).await
                            } else {
                                pipe.begin("tools/call", json!({"name":call.name,"arguments":call.args})).await
                            };
                            if let Ok(id) = begun {
                                pending.insert(id, (Some(call.reply), tokio::time::Instant::now()));
                            } else { let _ = call.reply.send(own(UNKNOWN, "TRANSPORT_LOST")); ended = "send failed"; break; }
                        }
                        _ = poll.tick(), if !pending.values().any(|(reply, _)| reply.is_none()) => {
                            if let Ok(id) = pipe.begin("tools/call", json!({"name":"chrome_status","arguments":{}})).await {
                                pending.insert(id, (None, tokio::time::Instant::now()));
                            } else { ended = "status poll send failed"; break }
                        }
                        _ = expiry.tick() => {
                            if pending.values().any(|(_, start)| start.elapsed() >= Duration::from_secs(35)) { ended = "a call outlived 35 s"; break; }
                        }
                    }
                }
                tracing::warn!(reason = ended, "Chrome Control connection ended");
                for (_, (reply, _)) in pending {
                    if let Some(reply) = reply {
                        let _ = reply.send(own(UNKNOWN, "TRANSPORT_LOST"));
                    }
                }
            }

            pipe.abort().await;
        }
        if let Some(core) = shared.upgrade() {
            core.chrome_control.owner.clear_connection();
            core.chrome_control
                .work
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .disconnected();
            core.chrome_control
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            publish(&core);
        }
        if *stop.borrow() {
            return;
        }
        // Refuse queued and new work for the whole backoff; traffic cannot shorten it or replay work.
        let until = tokio::time::Instant::now() + Duration::from_millis(backoff);
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                () = tokio::time::sleep_until(until) => break,
                call = requests.recv() => {
                    if let Some(call) = call { let _ = call.reply.send(own(vw_vocabulary::texts::CHROME_RESTARTING, "OWNER_CLOSED")); }
                    else { return; }
                }
            }
        }
        backoff = (backoff * 2).min(5000);
    }
}

async fn synchronize(
    shared: &Weak<Shared>,
    pipe: &mut super::pipe::Pipe,
    sent: &mut Value,
) -> Result<(), ()> {
    // Read trusted lowerings before sending any policy. An unchanged policy is never re-applied over one.
    pipe.notify("notifications/owner/drain", json!({})).await?;
    pipe.rpc("ping", json!({})).await?;
    super::switches::synchronize(shared.upgrade().ok_or(())?.as_ref(), pipe).await?;
    let next = mapping::policy();
    if *sent != next {
        pipe.notify("notifications/owner/policy", next.clone())
            .await?;
        pipe.rpc("ping", json!({})).await?;
        *sent = next;
    }
    Ok(())
}

#[cfg(test)]
#[path = "owner_pairing_tests.rs"]
mod pairing_tests;

#[cfg(test)]
#[path = "owner_disconnect_tests.rs"]
mod disconnect_tests;
