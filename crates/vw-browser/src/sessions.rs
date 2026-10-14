//! Per-session state of the agent browser: the latest observation's refs (what an action may target),
//! the user's Stop (a flag plus a cancellation counter an action in flight watches), and the action log the
//! Browser tab shows. The user's own clicks and typing in the page never enter the log: only `browser_act` calls do
//! (the agent's, labelled `agent`, the Debug API's, labelled `debugApi`, and the window's, labelled `user`).
//!
//! Depends on: serde, tokio (`watch`, `broadcast`). Called by: `driver`, vw-core-api (state, stop, resume, new turn).
//! Invariants: the log keeps at most [`LOG_CAP`] entries, each string bounded; Stop applies to the agent only (the
//! Debug API acts as the user); it lasts until `resume` or the session's next turn.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

/// Log entries kept per session.
pub const LOG_CAP: usize = 40;
/// An agent counts as "using the browser" this long after its last action (ms).
pub const ACTIVE_FOR_MS: u64 = 30_000;

/// Who ran an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Actor {
    /// The session's agent (`browser_act` through its credential).
    Agent,
    /// A Debug API client (acting as the user).
    DebugApi,
    /// The app's own window (the user).
    User,
}

/// How an action ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Done.
    Done,
    /// Refused before anything happened (policy, stale ref, secret field, not actionable, stopped).
    Refused,
    /// Failed while running.
    Failed,
    /// Cancelled by the user's Stop.
    Stopped,
}

/// One line of the action log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Increasing per session.
    pub seq: u32,
    /// When it ended (ms since the Unix epoch).
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub at_ms: u64,
    /// Who.
    pub by: Actor,
    /// The action (`click`, `navigate`, ...).
    pub action: String,
    /// What it acted on (role and name, or a URL; page text, at most 80 characters).
    pub target: String,
    /// How it ended.
    pub outcome: Outcome,
    /// A short result or reason (at most 160 characters).
    pub detail: String,
}

/// What the Browser tab shows about agent use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    /// The session.
    pub session_id: String,
    /// The agent acted within the last 30 s or is acting now ("Agent is using the browser").
    pub agent_active: bool,
    /// An agent action is running now.
    pub in_flight: bool,
    /// One authorized native click is waiting for the user to bring this window to the front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub pending_native_click: Option<String>,
    /// The user pressed Stop; agent actions are refused until Resume or the next turn.
    pub stopped: bool,
    /// The last agent action (ms since the epoch).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_agent_at_ms: Option<u64>,
    /// Recent actions, newest last.
    pub log: Vec<LogEntry>,
    /// Metadata-only receipts for acts authorized by browser permission or native confirmation.
    pub receipts: Vec<crate::delegation::DelegationReceipt>,
}

/// The latest observation's refs.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    /// The engine mount that answered; a released/replaced mount cannot reuse its refs.
    pub(crate) mount: vw_preview::MountId,
    pub(crate) snapshot: String,
    pub(crate) refs: HashSet<String>,
    /// Names of the refs that were listed (for the prompt guard's target check and the log).
    pub(crate) names: HashMap<String, (String, String)>,
    /// The observation itself (the "before" of the next action's receipt).
    pub(crate) model: crate::model::PageModel,
    /// The mount's finished loads when the observation was made (a fill needs the same document).
    pub(crate) loads: Option<u64>,
}

#[derive(Default)]
struct Session {
    ended: bool,
    seen: Option<Seen>,
    stopped: bool,
    in_flight: u32,
    authorizing: u32,
    pending_native_click: bool,
    last_agent_at: Option<u64>,
    log: VecDeque<LogEntry>,
    receipts: Vec<crate::delegation::DelegationReceipt>,
    seq: u32,
}

/// Every session's agent-browser state.
pub struct Sessions {
    inner: Mutex<HashMap<String, Session>>,
    cancel: Mutex<HashMap<String, watch::Sender<u64>>>,
    changes: broadcast::Sender<String>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            cancel: Mutex::new(HashMap::new()),
            changes: broadcast::channel(64).0,
        }
    }
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn clip(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

impl Sessions {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changed(&self, session_id: &str) {
        let _ = self.changes.send(session_id.to_owned());
    }

    /// Session ids whose browser state changed (agent started or ended an action, Stop, Resume, log).
    #[must_use]
    pub fn changes(&self) -> broadcast::Receiver<String> {
        self.changes.subscribe()
    }

    /// The Browser tab's view of a session.
    #[must_use]
    pub fn state(&self, session_id: &str) -> BrowserState {
        let now = now_ms();
        let sessions = self.lock();
        let s = sessions.get(session_id).filter(|session| !session.ended);
        let last = s.and_then(|s| s.last_agent_at);
        let in_flight = s.is_some_and(|s| s.in_flight > 0);
        BrowserState {
            session_id: session_id.to_owned(),
            agent_active: in_flight || last.is_some_and(|t| now.saturating_sub(t) < ACTIVE_FOR_MS),
            in_flight,
            pending_native_click: s
                .filter(|s| s.pending_native_click)
                .map(|_| crate::native_focus::FOCUS_REQUEST.to_owned()),
            stopped: s.is_some_and(|s| s.stopped),
            last_agent_at_ms: last,
            log: s
                .map(|s| s.log.iter().cloned().collect())
                .unwrap_or_default(),
            receipts: s.map(|s| s.receipts.clone()).unwrap_or_default(),
        }
    }

    /// The user's Stop: cancels the agent's action in flight and refuses further agent actions.
    pub fn stop(&self, session_id: &str) {
        self.lock()
            .entry(session_id.to_owned())
            .or_default()
            .stopped = true;
        let mut cancel = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
        cancel
            .entry(session_id.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .send_modify(|n| *n += 1);
        drop(cancel);
        self.changed(session_id);
    }

    /// Resume (the user's button, or the session's next turn).
    pub fn resume(&self, session_id: &str) {
        let was = self
            .lock()
            .get_mut(session_id)
            .is_some_and(|s| std::mem::replace(&mut s.stopped, false));
        if was {
            self.changed(session_id);
        }
    }

    /// True while the user's Stop holds.
    #[must_use]
    pub fn stopped(&self, session_id: &str) -> bool {
        self.lock().get(session_id).is_some_and(|s| s.stopped)
    }

    /// A receiver that changes when the user presses Stop for `session_id`.
    #[must_use]
    pub fn cancel_signal(&self, session_id: &str) -> watch::Receiver<u64> {
        let mut cancel = self.cancel.lock().unwrap_or_else(PoisonError::into_inner);
        cancel
            .entry(session_id.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    /// An agent action starts (the "using the browser" strip shows).
    pub fn begin(&self, session_id: &str, by: Actor) -> Flight<'_> {
        if by == Actor::Agent {
            let mut sessions = self.lock();
            let s = sessions.entry(session_id.to_owned()).or_default();
            if s.ended {
                return Flight {
                    sessions: self,
                    session_id: session_id.to_owned(),
                    by,
                };
            }
            s.in_flight += 1;
            s.last_agent_at = Some(now_ms());
            drop(sessions);
            self.changed(session_id);
        }
        Flight {
            sessions: self,
            session_id: session_id.to_owned(),
            by,
        }
    }

    /// Records an ended action; its `Flight` guard owns the in-flight count.
    pub fn end(
        &self,
        session_id: &str,
        by: Actor,
        action: &str,
        target: &str,
        outcome: Outcome,
        detail: &str,
    ) {
        let mut sessions = self.lock();
        let s = sessions.entry(session_id.to_owned()).or_default();
        if s.ended {
            return;
        }
        if by == Actor::Agent {
            s.last_agent_at = Some(now_ms());
        }
        s.seq += 1;
        let entry = LogEntry {
            seq: s.seq,
            at_ms: now_ms(),
            by,
            action: clip(action, 20),
            target: clip(target, 80),
            outcome,
            detail: clip(detail, 160),
        };
        s.log.push_back(entry);
        while s.log.len() > LOG_CAP {
            s.log.pop_front();
        }
        drop(sessions);
        self.changed(session_id);
    }

    // Save and append under the lifecycle lock: deletion first closes this boundary, then forgets durable receipts.
    // The trusted writer only touches its receipt store; it must not call back into Sessions.
    pub(crate) fn record_authorized(
        &self,
        receipt: crate::delegation::DelegationReceipt,
        save: impl FnOnce(&crate::delegation::DelegationReceipt) -> Result<(), String>,
    ) -> Result<(), String> {
        let session_id = receipt.session_id.clone();
        let mut sessions = self.lock();
        let Some(session) = sessions
            .get_mut(&session_id)
            .filter(|session| !session.ended)
        else {
            return Ok(());
        };
        let result = save(&receipt);
        session.receipts.push(receipt);
        drop(sessions);
        self.changed(&session_id);
        result
    }

    /// Restores trusted metadata-only receipts from the core's private app-data store at startup.
    pub fn restore_delegation_receipts(&self, receipts: Vec<crate::delegation::DelegationReceipt>) {
        for receipt in receipts {
            self.lock()
                .entry(receipt.session_id.clone())
                .or_default()
                .receipts
                .push(receipt);
        }
    }

    /// Records the latest observation's refs.
    pub(crate) fn set_seen(&self, session_id: &str, seen: Seen) {
        let mut sessions = self.lock();
        let session = sessions.entry(session_id.to_owned()).or_default();
        if !session.ended {
            session.seen = Some(seen);
        }
    }

    /// The latest observation's refs.
    pub(crate) fn seen(&self, session_id: &str) -> Option<Seen> {
        self.lock().get(session_id).and_then(|s| s.seen.clone())
    }

    /// Forgets the refs (the page was replaced by a navigation that did not come from an observation).
    pub fn forget_refs(&self, session_id: &str) {
        if let Some(s) = self.lock().get_mut(session_id) {
            s.seen = None;
        }
    }

    /// A session ended: its state goes.
    pub fn session_ended(&self, session_id: &str) {
        self.cancel_actions(session_id);
        // Keep only a metadata-free tombstone so canceled futures cannot recreate a deleted session.
        self.lock().insert(
            session_id.to_owned(),
            Session {
                ended: true,
                ..Session::default()
            },
        );
        self.cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session_id);
    }

    /// Cancels active work without setting the persistent user Stop flag (e.g. a finished or new turn).
    pub fn cancel_actions(&self, session_id: &str) {
        if let Some(sender) = self
            .cancel
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(session_id)
        {
            sender.send_modify(|n| *n += 1);
        }
    }

    /// A finished turn abandons its outstanding native question, while bounded input already running may finish.
    pub fn cancel_confirmations(&self, session_id: &str) {
        if self
            .lock()
            .get(session_id)
            .is_some_and(|session| session.authorizing > 0)
        {
            self.cancel_actions(session_id);
        }
    }

    /// Marks a host-held decision or queued tab close so turn end cancels its pending work.
    pub fn authorization_wait(&self, session_id: &str, by: Actor) -> impl Drop + Send + '_ {
        if by == Actor::Agent {
            self.lock()
                .entry(session_id.to_owned())
                .or_default()
                .authorizing += 1;
        }
        AuthorizationWait {
            sessions: self,
            session_id: session_id.to_owned(),
            by,
        }
    }

    pub(crate) fn queue_native_click(&self, session_id: &str) -> bool {
        let mut sessions = self.lock();
        let s = sessions.entry(session_id.to_owned()).or_default();
        if s.ended || s.pending_native_click {
            return false;
        }
        s.pending_native_click = true;
        drop(sessions);
        self.changed(session_id);
        true
    }

    pub(crate) fn clear_native_click(&self, session_id: &str) {
        if let Some(s) = self.lock().get_mut(session_id) {
            s.pending_native_click = false;
        }
        self.changed(session_id);
    }
}

/// Owns one in-flight count. Cancellation or dropping a caller clears activity just like normal completion.
#[must_use]
pub struct Flight<'a> {
    sessions: &'a Sessions,
    session_id: String,
    by: Actor,
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        if self.by == Actor::Agent {
            if let Some(s) = self.sessions.lock().get_mut(&self.session_id) {
                s.in_flight = s.in_flight.saturating_sub(1);
            }
            self.sessions.changed(&self.session_id);
        }
    }
}

/// Guard for a host-held browser decision or queued close.
struct AuthorizationWait<'a> {
    sessions: &'a Sessions,
    session_id: String,
    by: Actor,
}

impl Drop for AuthorizationWait<'_> {
    fn drop(&mut self) {
        if self.by == Actor::Agent
            && let Some(session) = self.sessions.lock().get_mut(&self.session_id)
        {
            session.authorizing = session.authorizing.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod close_wait_tests {
    use super::{Actor, Sessions};

    #[test]
    fn turn_end_cancels_a_pending_host_close_decision() {
        let sessions = Sessions::default();
        let mut cancel = sessions.cancel_signal("page");
        cancel.borrow_and_update();
        let wait = sessions.authorization_wait("page", Actor::Agent);
        sessions.cancel_confirmations("page");
        assert!(cancel.has_changed().unwrap());
        drop(wait);
        cancel.borrow_and_update();
        sessions.cancel_confirmations("page");
        assert!(!cancel.has_changed().unwrap());
    }
}
