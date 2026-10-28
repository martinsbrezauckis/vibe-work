//! App-lifetime owner of the helper's private pipes. Depends on tokio and the key service;
//! called by core startup, Settings and the checked gateway. Requests are never replayed after pipe loss.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::core::Shared;

use super::{mapping, setup};

pub(super) const TOOLS: [&str; 3] = ["chrome_status", "chrome_task", "chrome_execute"];
const UNKNOWN: &str =
    "Outcome unknown: Chrome Control's connection ended. Check Chrome before trying again.";

struct Call {
    name: String,
    args: Value,
    reply: oneshot::Sender<Value>,
}

struct Running {
    send: mpsc::Sender<Call>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct Owner {
    running: Mutex<Option<Running>>,
    pub(super) tools: Mutex<Vec<Value>>,
    pub(super) pairing: Mutex<(String, Option<String>)>,
}

impl Owner {
    pub(super) async fn stop(&self) {
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

    fn clear_connection(&self) {
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        *self.pairing.lock().unwrap_or_else(PoisonError::into_inner) =
            ("Chrome Control is not connected".to_owned(), None);
    }

    pub(super) async fn call(&self, name: &str, args: Value) -> Value {
        let send = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|run| run.send.clone());
        let Some(send) = send else {
            return failure("Not run: Chrome Control is not running.", "OWNER_REQUIRED");
        };
        let (reply, answer) = oneshot::channel();
        if send
            .send(Call {
                name: name.to_owned(),
                args,
                reply,
            })
            .await
            .is_err()
        {
            return failure(UNKNOWN, "TRANSPORT_LOST");
        }
        answer
            .await
            .unwrap_or_else(|_| failure(UNKNOWN, "TRANSPORT_LOST"))
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
    let Some(dir) = chrome.dir() else {
        return Ok(());
    };
    if setup::view(&chrome.config, Some(dir), &shared.instructions.own_home).state
        == setup::SetupState::NotSetUp
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
        .ok_or("Chrome Control needs the OS credential store.")?;
    let restrictions = keys
        .chrome_restrictions()
        .await
        .map_err(|_| "Could not read Chrome Control's saved permissions.")?;
    *chrome
        .lowered
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = restrictions;
    let secret = keys
        .chrome_owner()
        .await
        .map_err(|_| "Could not read Chrome Control's owner identity.")?
        .ok_or("Chrome Control needs Set up again to create its owner identity.")?;

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

fn policy(shared: &Shared) -> Value {
    let mut value = mapping::policy(
        shared.chrome_control.settings(),
        crate::browser_delegation::load(shared).close_user_tabs,
    );
    for (cap, lower) in shared
        .chrome_control
        .lowered
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
    {
        value["policy"][cap] = json!(lower);
    }
    value
}

pub(super) fn publish(shared: &Arc<Shared>) {
    let core = crate::Core {
        shared: shared.clone(),
    };
    shared.emit(crate::CoreEvent::ChromeControl {
        view: core.chrome_view(),
    });
}

pub(super) async fn observe(shared: &Weak<Shared>, message: &Value) -> Result<(), ()> {
    let Some(shared) = shared.upgrade() else {
        return Err(());
    };
    let chrome = &shared.chrome_control;
    if message["method"] == "notifications/owner/pairing" {
        if let Some(result) = pairing_result(&message["params"]) {
            update_pairing(&Arc::downgrade(&shared), &result);
        }
        return Ok(());
    }
    if message["method"] == "notifications/owner/policy_lowered" {
        let _held = chrome.lifecycle.lock().await;
        let params = &message["params"];
        let current = mapping::policy(
            chrome.settings(),
            crate::browser_delegation::load(&shared).close_user_tabs,
        );
        let saved = {
            let mut lower = chrome
                .lowered
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(rows) = params["policy"].as_object() {
                for (cap, value) in rows {
                    let rank = |v: &str| match v {
                        "allow" => 2,
                        "ask" => 1,
                        _ => 0,
                    };
                    if let Some(value) = value
                        .as_str()
                        .filter(|v| matches!(*v, "allow" | "ask" | "never"))
                    {
                        let before = lower.get(cap).map_or_else(
                            || current["policy"][cap].as_str().unwrap_or("never"),
                            String::as_str,
                        );
                        if rank(value) < rank(before) {
                            lower.insert(cap.clone(), value.to_owned());
                        }
                    }
                }
            }
            lower.clone()
        };
        if let Some(keys) = &shared.config.keys {
            keys.save_chrome_restrictions(saved).await.map_err(|_| ())?;
        }
        // A lowered level is also effective immediately. This never restores a saved raise.
        if let Ok(level) = serde_json::from_value(params["level"].clone()) {
            let mut state = chrome.current();
            if level < state.effective.level {
                state.effective.level = level;
                state.restore = None;
                if let Some(dir) = chrome.dir() {
                    let _ = super::store::save(dir, state.effective);
                }
            }
        }
        publish(&shared);
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
            "profiles":[{"connected":true}],"ownerFingerprint":fingerprint}})),
        "mismatch" => {
            Some(json!({"structuredContent":{"error":{"code":"OWNER_IDENTITY_MISMATCH"}}}))
        }
        _ => None,
    }
}

pub(super) fn update_pairing(shared: &Weak<Shared>, result: &Value) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let data = &result["structuredContent"];
    let code = data["error"]["code"].as_str().unwrap_or_default();
    let connected = data["profiles"]
        .as_array()
        .is_some_and(|p| p.iter().any(|p| p["connected"] == true));
    if data.get("profiles").is_none() && code.is_empty() {
        return;
    }
    let state = match code {
        "OWNER_IDENTITY_MISMATCH" | "OWNER_CONFLICT" => {
            "Chrome shows a different Vibe Work — pair again"
        }
        "OWNER_APPROVAL_REQUIRED" => "Waiting for you to pair in Chrome",
        _ if connected => "Paired",
        _ => "Waiting for you to pair in Chrome",
    };
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
    let next = (state.to_owned(), fingerprint);
    if *current != next {
        *current = next;
        drop(current);
        publish(&shared);
    }
}

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
        if let Ok(mut pipe) = result {
            if let Some(shared) = shared.upgrade() {
                shared
                    .chrome_control
                    .tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            }
            let mut sent_policy = Value::Null;
            let mut poll = tokio::time::interval(Duration::from_secs(2));
            loop {
                tokio::select! {
                    biased;
                    _ = stop.changed() => { pipe.close().await; return; }
                    _ = pipe.child.wait() => break,
                    call = requests.recv() => {
                        let Some(call) = call else { pipe.close().await; return };
                        if call.reply.is_closed() { continue; }
                        let operation = async {
                            synchronize(&shared, &mut pipe, &mut sent_policy).await?;
                            pipe.rpc("tools/call", json!({"name":call.name,"arguments":call.args})).await
                        };
                        let result = tokio::select! {
                            _ = stop.changed() => Err(()),
                            result = operation => result,
                        };
                        if let Ok(result) = result { update_pairing(&shared, &result); let _ = call.reply.send(result); }
                        else { let _ = call.reply.send(failure(UNKNOWN, "TRANSPORT_LOST")); break; }
                    }
                    _ = poll.tick() => {
                        let operation = async {
                            synchronize(&shared, &mut pipe, &mut sent_policy).await?;
                            pipe.rpc("tools/call", json!({"name":"chrome_status","arguments":{}})).await
                        };
                        let result = tokio::select! {
                            _ = stop.changed() => Err(()),
                            result = operation => result,
                        };
                        match result { Ok(result) => update_pairing(&shared, &result), Err(()) => break }
                    }
                }
            }
            pipe.close().await;
        }
        if let Some(core) = shared.upgrade() {
            core.chrome_control.owner.clear_connection();
            publish(&core);
        }
        // Refuse queued and new work for the whole backoff; traffic cannot shorten it or replay work.
        let until = tokio::time::Instant::now() + Duration::from_millis(backoff);
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                () = tokio::time::sleep_until(until) => break,
                call = requests.recv() => {
                    if let Some(call) = call { let _ = call.reply.send(failure("Not run: Chrome Control is restarting. Ask the user to check Chrome.", "OWNER_CLOSED")); }
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
    let next = shared.upgrade().map(|core| policy(&core)).ok_or(())?;
    if *sent != next {
        pipe.notify("notifications/owner/policy", next.clone())
            .await?;
        pipe.rpc("ping", json!({})).await?;
        *sent = next;
    }
    Ok(())
}

#[cfg(test)]
mod pairing_tests {
    use super::pairing_result;
    use serde_json::json;

    const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn waiting_and_paired_show_our_fingerprint() {
        let waiting =
            pairing_result(&json!({"profileId":"p","state":"waiting","fingerprint":OURS})).unwrap();
        assert_eq!(
            waiting["structuredContent"]["error"]["code"],
            "OWNER_APPROVAL_REQUIRED"
        );
        assert_eq!(waiting["structuredContent"]["ownerFingerprint"], OURS);
        let paired =
            pairing_result(&json!({"profileId":"p","state":"paired","fingerprint":OURS})).unwrap();
        assert_eq!(
            paired["structuredContent"]["profiles"][0]["connected"],
            true
        );
        assert_eq!(paired["structuredContent"]["ownerFingerprint"], OURS);
    }

    #[test]
    fn mismatch_says_pair_again_and_never_takes_the_other_fingerprint() {
        let mismatch =
            pairing_result(&json!({"profileId":"p","state":"mismatch","fingerprint":OTHER}))
                .unwrap();
        assert_eq!(
            mismatch["structuredContent"]["error"]["code"],
            "OWNER_IDENTITY_MISMATCH"
        );
        assert!(
            mismatch["structuredContent"]
                .get("ownerFingerprint")
                .is_none()
        );
    }

    #[test]
    fn unknown_or_missing_state_is_ignored() {
        assert!(pairing_result(&json!({"state":"admit"})).is_none());
        assert!(pairing_result(&json!({})).is_none());
    }
}

#[cfg(test)]
mod disconnect_tests {
    use super::*;

    #[tokio::test]
    async fn explicit_stop_forgets_pairing_and_fingerprint() {
        let owner = Owner::default();
        *owner.pairing.lock().unwrap() = ("Paired".to_owned(), Some("0123456789abcdef".to_owned()));
        owner.stop().await;
        assert_eq!(
            *owner.pairing.lock().unwrap(),
            ("Chrome Control is not connected".to_owned(), None)
        );
    }
}
