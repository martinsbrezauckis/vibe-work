//! Per-session resume barriers, controlled only by Core through the credential provider.
//! Provider metadata identifies the session and user entry; only trusted fresh-turn events
//! authorize an entry after the old turn ended. HTTP callers cannot advance this generation.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;
use vw_supervisor::{ModelRequestHold, ModelRequests, SecretError};

pub(crate) const ENDED: &str = "This model request belongs to a previous turn that was stopped on resume. Do not retry it; wait for the user's new message.";
const EXPIRED: &str = "The resumed turn could not be stopped in time. Restart Vibe Work, then send your message again.";
const CANCELLED: &str = "The chat was stopped while it was resuming. Do not continue the previous turn; send a new message to try again.";
const CLOSING: &str =
    "Vibe Work is closing. This model request was stopped; try again after restart.";
const ABANDONED: &str =
    "The chat could not finish resuming. Restart Vibe Work, then send your message again.";
const UNKNOWN: &str = "This model request has no verified fresh turn after resume. Do not retry the previous turn; send a new message in Vibe Work.";

#[derive(Clone, Copy)]
enum Phase {
    Holding,
    Open,
    Failed(&'static str),
}

struct State {
    phase: Phase,
    old_turn: String,
    old_inputs: HashSet<String>,
    turns: HashSet<String>,
    inputs: HashMap<String, String>,
}

struct Fence {
    state: watch::Sender<State>,
    deadline: Instant,
}

impl Fence {
    fn fail(&self, reason: &'static str) {
        self.state.send_modify(|state| {
            if matches!(state.phase, Phase::Holding) {
                state.phase = Phase::Failed(reason);
            }
        });
    }
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, Arc<Fence>>,
    closing: bool,
}

/// Shared with the token slot so the server and its credential provider cannot use different barriers.
#[derive(Default)]
pub(crate) struct ResumeRequests(Mutex<Registry>);

impl ResumeRequests {
    /// A new server token invalidates every old guard and request, including requests already waiting.
    pub(crate) fn reset(&self) {
        let mut registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for fence in registry.sessions.values() {
            fence
                .state
                .send_modify(|s| s.phase = Phase::Failed(ABANDONED));
        }
        registry.sessions.clear();
        registry.closing = false;
    }

    /// Capture a barrier for this request; unknown attribution fails closed once resumes are guarded.
    pub(crate) fn admission(
        &self,
        session: Option<&str>,
        input: Option<&str>,
    ) -> Result<Option<Admission>, &'static str> {
        let registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if registry.closing {
            return Err(CLOSING);
        }
        let Some(session) = session.filter(|id| !id.is_empty()) else {
            return if registry.sessions.is_empty() {
                Ok(None)
            } else {
                Err(UNKNOWN)
            };
        };
        Ok(registry.sessions.get(session).map(|fence| Admission {
            fence: Arc::clone(fence),
            input: input.filter(|id| !id.is_empty()).map(str::to_owned),
        }))
    }
}

pub(crate) struct Admission {
    fence: Arc<Fence>,
    input: Option<String>,
}

impl Admission {
    pub(crate) fn holding(&self) -> bool {
        matches!(self.fence.state.borrow().phase, Phase::Holding)
    }

    /// The request keeps its original identity across release, retries, key waits and delayed bodies.
    pub(crate) async fn wait(self) -> Result<(), &'static str> {
        let mut state = self.fence.state.subscribe();
        // Fresh-turn notifications can trail HTTP delivery. This wait applies only to guarded chats.
        let observed_by = Instant::now() + Duration::from_secs(10);
        loop {
            let deadline = {
                let state = state.borrow_and_update();
                match state.phase {
                    Phase::Holding => self.fence.deadline,
                    Phase::Failed(reason) => return Err(reason),
                    Phase::Open => {
                        let Some(input) = &self.input else {
                            return Err(UNKNOWN);
                        };
                        if state.old_inputs.contains(input) {
                            return Err(ENDED);
                        }
                        if state
                            .inputs
                            .get(input)
                            .is_some_and(|turn| state.turns.contains(turn))
                        {
                            return Ok(());
                        }
                        observed_by
                    }
                }
            };
            if tokio::time::timeout_at(deadline, state.changed())
                .await
                .is_err()
            {
                if self.holding() {
                    self.fence.fail(EXPIRED);
                    return Err(EXPIRED);
                }
                return Err(ENDED);
            }
        }
    }
}

struct Hold {
    fence: Arc<Fence>,
    released: bool,
}

impl ModelRequestHold for Hold {
    fn release(&mut self) -> Result<(), SecretError> {
        let mut result = Ok(());
        self.fence.state.send_modify(|state| match state.phase {
            Phase::Holding if Instant::now() < self.fence.deadline => state.phase = Phase::Open,
            Phase::Holding => {
                state.phase = Phase::Failed(EXPIRED);
                result = Err(SecretError(EXPIRED.into()));
            }
            Phase::Failed(CANCELLED) => {
                // The caller has now proved termination. Refuse this cancelled Send but admit
                // a later fresh turn; requests already woken by Stop keep their refusal.
                state.phase = Phase::Open;
                result = Err(SecretError(CANCELLED.into()));
            }
            Phase::Failed(reason) => result = Err(SecretError(reason.into())),
            Phase::Open => {}
        });
        self.released = result.is_ok();
        result
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        if !self.released {
            self.fence.fail(ABANDONED);
        }
    }
}

impl ModelRequests for ResumeRequests {
    fn hold(
        &self,
        session: &str,
        old_turn: &str,
        old_inputs: &[String],
        timeout: Duration,
    ) -> Result<Box<dyn ModelRequestHold>, SecretError> {
        let mut registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if registry.closing {
            return Err(SecretError(CLOSING.into()));
        }
        let fence = Arc::new(Fence {
            state: watch::Sender::new(State {
                phase: Phase::Holding,
                old_turn: old_turn.to_owned(),
                old_inputs: old_inputs.iter().cloned().collect(),
                turns: HashSet::new(),
                inputs: HashMap::new(),
            }),
            deadline: Instant::now() + timeout.min(Duration::from_secs(60)),
        });
        if let Some(old) = registry
            .sessions
            .insert(session.to_owned(), Arc::clone(&fence))
        {
            old.state.send_modify(|s| s.phase = Phase::Failed(ENDED));
        }
        Ok(Box::new(Hold {
            fence,
            released: false,
        }))
    }

    fn new_turn(&self, session: &str, turn: &str, input: Option<&str>) {
        let registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(fence) = registry.sessions.get(session) {
            fence.state.send_modify(|state| {
                if turn != state.old_turn {
                    state.turns.insert(turn.to_owned());
                    if let Some(input) = input {
                        state.inputs.insert(input.to_owned(), turn.to_owned());
                    }
                }
            });
        }
    }

    fn user_input(&self, session: &str, turn: &str, input: &str) {
        let registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(fence) = registry.sessions.get(session) {
            fence.state.send_modify(|state| {
                if turn != state.old_turn {
                    state.inputs.insert(input.to_owned(), turn.to_owned());
                }
            });
        }
    }

    fn stop(&self, session: &str) {
        let registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(fence) = registry.sessions.get(session) {
            fence.fail(CANCELLED);
        }
    }

    fn shutdown(&self) {
        let mut registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        registry.closing = true;
        for fence in registry.sessions.values() {
            fence
                .state
                .send_modify(|s| s.phase = Phase::Failed(CLOSING));
        }
    }
}
