//! App-lifetime owner of the helper's private pipes. Depends on tokio and the key service;
//! called by core startup, Settings and the checked gateway. Requests are never replayed after pipe loss.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::core::Shared;

use super::{PairingState, mapping, setup};

#[path = "owner_observe.rs"]
mod observation;
#[cfg(test)]
use observation::pairing_result;
#[path = "owner_connection.rs"]
mod connection;
use connection::start_inner;
#[cfg(test)]
use connection::supervise;
#[cfg(test)]
pub(super) use observation::update_work_for_test;
pub(super) use observation::{
    observe, publish, publish_setup_if_changed, update_pairing, update_work,
};

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
    /// Sent by [`Owner::control`], which never waits for the answer: it is delivered although nobody holds its reply.
    detached: bool,
    admission: Option<crate::ask::grants::Admission>,
}

impl Call {
    async fn dispatch<T>(&self, send: impl std::future::Future<Output = T>) -> Option<T> {
        // The grant is checked while its dispatch gate is held. Revoke closes the grant and
        // waits for this guard before reporting completion to the person.
        let _guard = match &self.admission {
            Some(token)
                if !matches!(
                    self.name.as_str(),
                    "chrome_status" | "owner.primitiveResult"
                ) =>
            {
                Some(token.dispatch_guard().await)
            }
            _ => None,
        };
        if self.revoked() {
            None
        } else {
            Some(send.await)
        }
    }

    fn revoked(&self) -> bool {
        // Status and receipt reads finish an effect already sent; blocking them would turn an
        // in-flight outcome into an incorrect "not_run" receipt after revoke.
        !matches!(
            self.name.as_str(),
            "chrome_status" | "owner.primitiveResult"
        ) && self.admission.as_ref().is_some_and(|token| !token.allows())
    }

    /// Whether the owner still sends this call to the helper. A call whose caller stopped waiting (its reply is
    /// closed) is dropped, except fire-and-forget control calls (frame, idle, stop, delete, cancel), which carry
    /// the app's intent and must arrive.
    fn admitted(&self) -> bool {
        self.detached
            || !self.reply.is_closed()
            || (self.name == "chrome_session"
                && matches!(
                    self.args["action"].as_str(),
                    Some("stop" | "delete" | "cancel")
                ))
    }
}

#[cfg(test)]
mod revoke_dispatch_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoke_waits_for_owner_pipe_send_commitment() {
        let grants = Arc::new(crate::ask::grants::Grants::default());
        let card = json!({"vwToolAsk":true,"detail":{"kind":"approval","vwBrowserTier":2,
            "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":"t","steps":[
                {"action":"auth.fill","args":{"ref":"u2","login":"Demo"}}]}},"requiredPermissions":[]}});
        grants.note_callback("card", "chat", "chat", crate::ask::grants::scope_of(&card));
        let grant = grants
            .keep(grants.claim("card").unwrap(), vw_ask::AnsweredBy::Person, 1)
            .unwrap();
        let admission = grants.admission(&grant.id).unwrap();
        let (reply, _answer) = oneshot::channel();
        let call = Call {
            name: "chrome_execute".into(),
            args: json!({}),
            reply,
            rpc: false,
            detached: false,
            admission: Some(admission),
        };
        let (checked, reached) = oneshot::channel();
        let (resume, resumed) = oneshot::channel();
        let revoke_returned = Arc::new(AtomicBool::new(false));
        let late_send = Arc::new(AtomicBool::new(false));
        let returned = revoke_returned.clone();
        let late = late_send.clone();
        let owner = tokio::spawn(async move {
            call.dispatch(async move {
                checked.send(()).unwrap();
                resumed.await.unwrap();
                late.store(returned.load(Ordering::SeqCst), Ordering::SeqCst);
            })
            .await
        });
        reached.await.unwrap(); // Owner passed its final grant check, before the pipe write.
        let id = grant.id.clone();
        let revoke_grants = grants.clone();
        let returned = revoke_returned.clone();
        let mut revoke = tokio::spawn(async move {
            assert!(revoke_grants.remove_wait(&id).await);
            returned.store(true, Ordering::SeqCst);
        });
        let early = tokio::time::timeout(Duration::from_millis(100), &mut revoke).await;
        resume.send(()).unwrap();
        owner.await.unwrap();
        let returned_before_send = early.is_ok();
        if let Ok(result) = early {
            result.unwrap();
        } else {
            revoke.await.unwrap();
        }
        assert!(
            !returned_before_send,
            "Revoke returned while the owner had not committed the pipe send"
        );
        assert!(
            !late_send.load(Ordering::SeqCst),
            "a helper send followed completed Revoke"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_chats_revoke_does_not_take_another_chats_dispatch_gate() {
        let grants = Arc::new(crate::ask::grants::Grants::default());
        let card = json!({"vwToolAsk":true,"detail":{"kind":"approval","vwBrowserTier":2,
            "effect":{"toolName":"mcp_vw.chrome_execute","input":{"sessionId":"t","steps":[
                {"action":"auth.fill","args":{"ref":"u2","login":"Demo"}}]}},"requiredPermissions":[]}});
        let mut ids = Vec::new();
        for chat in ["first", "second"] {
            grants.note_callback(chat, chat, chat, crate::ask::grants::scope_of(&card));
            ids.push(
                grants
                    .keep(grants.claim(chat).unwrap(), vw_ask::AnsweredBy::Person, 1)
                    .unwrap()
                    .id,
            );
        }
        let first = grants.admission(&ids[0]).unwrap();
        let second = grants.admission(&ids[1]).unwrap();
        let (reply, _answer) = oneshot::channel();
        let first_call = Call {
            name: "chrome_execute".into(),
            args: json!({}),
            reply,
            rpc: false,
            detached: false,
            admission: Some(first.clone()),
        };
        let (at_send, reached) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let owner = tokio::spawn(async move {
            first_call
                .dispatch(async {
                    at_send.send(()).unwrap();
                    released.await.unwrap();
                })
                .await
        });
        reached.await.unwrap();
        let first_id = ids[0].clone();
        let revoking = grants.clone();
        let revoke = tokio::spawn(async move { revoking.remove_wait(&first_id).await });
        tokio::time::timeout(Duration::from_millis(100), async {
            while first.allows() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first chat's grant was revoked");
        let (reply, _answer) = oneshot::channel();
        let second_call = Call {
            name: "chrome_execute".into(),
            args: json!({}),
            reply,
            rpc: false,
            detached: false,
            admission: Some(second),
        };
        assert_eq!(
            tokio::time::timeout(
                Duration::from_millis(100),
                second_call.dispatch(async { 7 })
            )
            .await
            .unwrap(),
            Some(7),
            "another chat waited on the revoked chat's send"
        );
        release.send(()).unwrap();
        owner.await.unwrap();
        assert!(revoke.await.unwrap());
        let (reply, _answer) = oneshot::channel();
        let revoked_call = Call {
            name: "chrome_execute".into(),
            args: json!({}),
            reply,
            rpc: false,
            detached: false,
            admission: Some(first),
        };
        assert_eq!(revoked_call.dispatch(async { 9 }).await, None);
    }
}

/// An owner answer and who wrote it. The helper's answers can carry page-derived text (titles, URLs, page text and
/// errors, console lines, dialog text, download names), so they reach the model inside the untrusted-content envelope
/// (`content::for_model`). The core's own failures (not running, busy, restarting, connection lost) hold only Vibe
/// Work's fixed words and reach the model as the tool's own error, outside it.
pub(super) enum Answer {
    Helper(Value),
    Own(Value),
}

/// A private helper refusal is different from an unavailable pipe. Page plans inspect only vetted
/// helper messages; arbitrary error text may contain page data and must not reach a chat verbatim.
pub(super) enum RpcFailure {
    Unavailable,
    Revoked,
    Rejected {
        code: i64,
        helper_code: Option<String>,
        message: String,
    },
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

type SetupSnapshot = super::ChromeControlView;

#[derive(Default)]
pub(super) struct Owner {
    pub(super) approvals: super::pairing_approval::Approvals,
    running: Mutex<Option<Running>>,
    pub(super) tools: Mutex<Vec<Value>>,
    pub(super) pairing: Mutex<(PairingState, Option<String>)>,
    pub(super) safari_pairing: Mutex<(String, Option<String>)>,
    /// Safari pairing states received from the helper; a change means an extension port is open.
    pub(super) safari_seen: std::sync::atomic::AtomicU64,
    /// When the last Safari pairing state arrived (the extension's page was running then).
    pub(super) safari_seen_at: Mutex<Option<std::time::Instant>>,
    /// The person approved a Safari pairing and the extension has not connected since (`safari::view`).
    pub(super) safari_approved: std::sync::atomic::AtomicBool,
    /// Times a completed Safari pairing asked macOS for its Accessibility prompt (once per pairing).
    pub(super) safari_accessibility_prompts: std::sync::atomic::AtomicU64,
    pub(super) safari_error: Mutex<Option<super::safari::ConnectionError>>,
    pub(super) firefox_pairing: Mutex<(PairingState, Option<String>)>,
    pub(super) firefox_share_pairing: Mutex<(PairingState, Option<String>)>,
    pub(super) firefox_states: Mutex<Vec<String>>,
    pub(super) edge_pairing: Mutex<(PairingState, Option<String>)>,
    observed_setup: Mutex<Option<SetupSnapshot>>,
    pub(super) credential_missing: std::sync::atomic::AtomicBool,
    /// The owner identity could not be read because this computer has no usable system keyring. Cleared when a
    /// later start reads it; it is not a missing credential, so Remove does not apply.
    pub(super) keyring_unavailable: std::sync::atomic::AtomicBool,
    /// The last helper launch was refused because another helper holds this state folder's lock
    /// (`OWNER_CONFLICT`). The owner keeps retrying; a helper connection or `stop` clears it.
    pub(super) helper_conflict: std::sync::atomic::AtomicBool,
    pub(super) connected_profiles: super::addon::ConnectedProfiles,
    pub(super) profile_ids: super::profile_ids::ProfileIds,
    pub(super) wake: super::reconnect::Wake,
    pub(super) not_ready: super::reconnect::NotReady,
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
                detached: true,
                admission: None,
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
        self.answering_after(Duration::ZERO, answer)
    }

    /// Deterministic delayed helper replies for deadline tests, through the real owner request queue.
    #[cfg(test)]
    pub(super) fn answering_after(
        &self,
        delay: Duration,
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
                // The real owner loop's admission rule, so tests see exactly what reaches the helper.
                if !call.admitted() {
                    continue;
                }
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                if call.revoked() {
                    let _ = call.reply.send(own(
                        crate::ask::tool_calls::REVOKED_BROWSER_GRANT,
                        "CONSENT_REVOKED",
                    ));
                    continue;
                }
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
        self.helper_conflict
            .store(false, std::sync::atomic::Ordering::SeqCst);
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
        self.clear_firefox();
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

    /// One owner request on the private pipe (`method` is not a tool). Older callers treat a helper
    /// refusal like an unavailable method; page plans use `rpc_result` to retain its reason.
    pub(super) async fn rpc(&self, method: &str, params: Value) -> Option<Value> {
        self.rpc_result(method, params).await.ok()
    }

    pub(super) async fn rpc_result(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcFailure> {
        let send = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|run| run.send.clone())
            .ok_or(RpcFailure::Unavailable)?;
        match Self::request(send, method, params, true).await {
            Some(Answer::Helper(value)) => {
                if let Some(error) = value.get("rpcError") {
                    Err(RpcFailure::Rejected {
                        code: error["code"].as_i64().unwrap_or_default(),
                        helper_code: error["data"]["code"].as_str().map(str::to_owned),
                        message: error["message"].as_str().unwrap_or_default().to_owned(),
                    })
                } else {
                    Ok(value)
                }
            }
            Some(Answer::Own(value))
                if value["structuredContent"]["error"]["code"] == "CONSENT_REVOKED" =>
            {
                Err(RpcFailure::Revoked)
            }
            _ => Err(RpcFailure::Unavailable),
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
            return own(
                &format!(
                    "{} Retry chrome_status in a few seconds. Ask the user to restart Vibe Work if it remains unavailable.",
                    vw_vocabulary::texts::CHROME_NOT_RUNNING
                ),
                "OWNER_REQUIRED",
            );
        };
        Self::request(send, name, args, false)
            .await
            .unwrap_or_else(|| {
                if super::deadline::expired() {
                    own(super::deadline::EXPIRED, "BROWSER_STEP_TIMEOUT")
                } else {
                    own(UNKNOWN, "TRANSPORT_LOST")
                }
            })
    }

    /// Bound queue admission and the answer together: startup may never reach the request dispatcher.
    async fn request(
        send: mpsc::Sender<Call>,
        name: &str,
        args: Value,
        rpc: bool,
    ) -> Option<Answer> {
        let deadline = super::deadline::until(Duration::from_secs(35));
        if deadline <= tokio::time::Instant::now() {
            return None;
        }
        tokio::time::timeout_at(deadline, async {
            let (reply, answer) = oneshot::channel();
            send.send(Call {
                name: name.to_owned(),
                args,
                reply,
                rpc,
                detached: false,
                admission: super::batch::admission(),
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

/// Uses the same default endpoint as Chrome's native host. Scratch homes select isolated disposable endpoints. A
/// development build on macOS that carries the Safari parts keeps its own folder (`macos_state_dir`), so it never
/// rewrites the installed, signed app's pairing, pins or socket; the signed release folder is unchanged.
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
        // Safari reaches the helper through its App Group socket, never through this folder, so a Safari build can
        // move it; Chrome's native host (started by Chrome without arguments) always uses the default one.
        let own = config.dev_build && super::safari_package::installed();
        std::env::var_os("HOME").map(|home| macos_state_dir(Path::new(&home), own))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
            .map(|p| p.join("vibework/chrome-control"))
    }
    .filter(|p| p.is_absolute())
}

/// The macOS helper state folder under `home`: the signed release's `VibeWork/ChromeControl`, or, for a development
/// build with the Safari parts (`own_dev_folder`), `VibeWork/ChromeControl-dev` beside it.
pub(super) fn macos_state_dir(home: &Path, own_dev_folder: bool) -> PathBuf {
    let folder = if own_dev_folder {
        "ChromeControl-dev"
    } else {
        "ChromeControl"
    };
    home.join("Library/Application Support/VibeWork")
        .join(folder)
}

#[cfg(test)]
#[path = "state_dir_tests.rs"]
mod state_dir_tests;

pub(crate) async fn start(shared: &Arc<Shared>) -> Result<(), String> {
    start_inner(shared, false).await
}

pub(super) async fn start_for_removal(shared: &Arc<Shared>) -> Result<(), String> {
    start_inner(shared, true).await
}

#[cfg(test)]
#[path = "owner_pairing_tests.rs"]
mod pairing_tests;

#[cfg(test)]
#[path = "owner_disconnect_tests.rs"]
mod disconnect_tests;

/// The keyring sentence (a vocabulary template: `{yourBrowser}` is worded by the caller) while the owner identity
/// cannot be read for lack of a system keyring. Tries once to start the helper again first, so a keyring the person
/// has set up since is used at once and the flag cannot outlive its cause.
pub(super) async fn keyring_blocked(shared: &Arc<Shared>) -> Option<&'static str> {
    let owner = &shared.chrome_control.owner;
    if !owner
        .keyring_unavailable
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return None;
    }
    let _ = start_inner(shared, false).await;
    owner
        .keyring_unavailable
        .load(std::sync::atomic::Ordering::SeqCst)
        .then_some(vw_vocabulary::texts::CHROME_OWNER_STORE_UNAVAILABLE)
}
