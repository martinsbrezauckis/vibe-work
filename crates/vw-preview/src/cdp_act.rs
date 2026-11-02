//! Completion-driven act controls for `CdpFrames`. Serializes generations, includes newly attached sessions,
//! and confirms cleanup or tears down the affected session. Uses the registry's transport on the UI thread.
use super::{CdpFrames, auto_attach};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

pub type Done = Box<dyn FnOnce(Result<(), String>)>;
const PICKER: &str = "Page.setInterceptFileChooserDialog";
const FOCUS: &str = "Emulation.setFocusEmulationEnabled";
const REFUSED: &str = "Not run: browser input restrictions could not be confirmed. Reopen the browser view before trying again.";

#[derive(Default)]
pub(super) struct Act {
    pub(super) generation: u64,
    pub(super) enabled: bool,
    pub(super) running: bool,
    closed: bool,
    confirmed: BTreeSet<String>,
    done: Option<Done>,
    failure: Option<String>,
}

impl CdpFrames {
    /// Confirms there are no pending or failed act controls before the final native dispatch.
    ///
    /// # Errors
    /// Act restrictions are pending, ended or failed. A registry used only for reads has no act window.
    pub fn act_ready(&self) -> Result<(), String> {
        let state = self.state.borrow();
        let act = &state.act;
        if act.generation != 0
            && (!act.enabled || act.running || act.closed || act.failure.is_some())
        {
            Err(REFUSED.into())
        } else {
            Ok(())
        }
    }

    /// A timed-out native operation cannot leave emulation or interception behind. Newer acts keep their view.
    pub fn abort_act(&self, generation: u64, done: Done) {
        if self.state.borrow().act.generation == generation {
            self.close_act_view(done);
        } else {
            done(Ok(()));
        }
    }

    /// Checks one full-screen exit completion against the native view state. True means the caller must retry
    /// (including a failed evaluation or state read); false means the view has actually left full screen.
    pub fn exit_full_screen(
        &self,
        contains: Rc<dyn Fn() -> Result<bool, String>>,
        done: Box<dyn FnOnce(Result<bool, String>)>,
    ) {
        match contains() {
            Ok(false) => {
                done(Ok(false));
                return;
            }
            Err(error) => {
                tracing::debug!("browser full-screen state unconfirmed: {error}");
                done(Ok(true));
                return;
            }
            Ok(true) => {}
        }
        let mut frames = self.clone();
        frames.await_promise = true;
        frames.evaluate(
            None,
            "document.fullscreenElement ? document.exitFullscreen().then(() => true) : false",
            Box::new(move |result| {
                if let Err(error) = result {
                    tracing::debug!("browser full-screen exit will recheck native state: {error}");
                }
                done(Ok(contains().unwrap_or_else(|error| {
                    tracing::debug!("browser full-screen state unconfirmed: {error}");
                    true
                })));
            }),
        );
    }

    /// Admission waits for the picker, focus and attachment controls in every current session.
    pub fn start_act(&self, generation: u64, done: Done) {
        let previous = {
            let mut state = self.state.borrow_mut();
            let act = &mut state.act;
            if act.closed || generation < act.generation {
                drop(state);
                done(Err(REFUSED.into()));
                return;
            }
            act.generation = generation;
            act.enabled = true;
            act.confirmed.clear();
            act.failure = None;
            act.done.replace(done)
        };
        if let Some(previous) = previous {
            previous(Err("Not run: a newer browser act replaced this one. Observe the page before trying again.".into()));
        }
        self.sync_act();
    }

    /// Ends only this generation. Each failed disable is retried once, then its session is detached.
    pub fn end_act(&self, generation: u64, done: Done) {
        let previous = {
            let mut state = self.state.borrow_mut();
            let act = &mut state.act;
            if generation != act.generation {
                drop(state);
                done(Ok(()));
                return;
            }
            if !act.enabled {
                if let Some(previous) = act.done.take() {
                    act.done = Some(Box::new(move |result| {
                        previous(result.clone());
                        done(result);
                    }));
                    return;
                }
                if !act.running {
                    drop(state);
                    done(Ok(()));
                    return;
                }
            }
            act.enabled = false;
            act.confirmed.clear();
            act.done.replace(done)
        };
        if let Some(previous) = previous {
            previous(Err(REFUSED.into()));
        }
        self.sync_act();
    }

    fn current_act(&self, generation: u64, enabled: bool) -> bool {
        let state = self.state.borrow();
        state.act.generation == generation && state.act.enabled == enabled && !state.act.closed
    }

    /// Active recovery shares the control round; ordinary reads after cleanup only restore attachment.
    pub(super) fn recover_attachment(&self, done: Done) {
        let mut state = self.state.borrow_mut();
        if state.act.generation == 0
            || !state.act.enabled
                && !state.act.running
                && state.act.done.is_none()
                && !state.act.closed
        {
            let failure = state.act.failure.clone();
            drop(state);
            if let Some(error) = failure {
                done(Err(error));
                return;
            }
            self.transport.call(
                "",
                "Target.setAutoAttach",
                auto_attach(),
                Box::new(move |r| done(r.map(drop))),
            );
            return;
        }
        state.act.confirmed.remove("");
        let previous = state.act.done.take();
        state.act.done = Some(Box::new(move |result| {
            if let Some(previous) = previous {
                previous(result.clone());
            }
            done(result);
        }));
        drop(state);
        self.sync_act();
    }

    /// Only one round is in flight. A newer generation waits for that round, then reapplies its own controls;
    /// obsolete callbacks cannot send a retry or detach a session belonging to the newer act.
    pub(super) fn sync_act(&self) {
        let (generation, enabled, sessions) = {
            let mut state = self.state.borrow_mut();
            if state.act.generation == 0
                || state.act.running
                || !state.act.enabled && state.act.done.is_none()
            {
                return;
            }
            let sessions: Vec<_> = std::iter::once(String::new())
                .chain(state.sessions.iter().cloned())
                .filter(|s| !state.act.confirmed.contains(s))
                .collect();
            if sessions.is_empty() || state.act.closed {
                let done = state.act.done.take();
                let result = state.act.failure.clone().map_or(Ok(()), Err);
                drop(state);
                if let Some(done) = done {
                    done(result);
                }
                return;
            }
            state.act.running = true;
            (state.act.generation, state.act.enabled, sessions)
        };
        let pending = Rc::new(RefCell::new((sessions.len(), None::<String>)));
        for session in sessions {
            let (frames, pending, owner) = (self.clone(), pending.clone(), session.clone());
            self.control_session(
                &session,
                generation,
                enabled,
                0,
                false,
                Box::new(move |result| {
                    let finished = {
                        let mut pending = pending.borrow_mut();
                        pending.0 -= 1;
                        if let Err(error) = result {
                            pending.1.get_or_insert(error);
                        }
                        pending.0 == 0
                    };
                    {
                        let mut state = frames.state.borrow_mut();
                        if state.act.generation == generation && state.act.enabled == enabled {
                            state.act.confirmed.insert(owner);
                        }
                        if finished {
                            state.act.running = false;
                            if state.act.generation == generation
                                && state.act.enabled == enabled
                                && let Some(error) = pending.borrow_mut().1.take()
                            {
                                state.act.failure.get_or_insert(error);
                                if enabled {
                                    state.act.enabled = false;
                                    state.act.confirmed.clear();
                                }
                            }
                        }
                    }
                    if finished {
                        frames.sync_act();
                    }
                }),
            );
        }
    }

    fn control_session(
        &self,
        session: &str,
        generation: u64,
        enabled: bool,
        step: u8,
        retried: bool,
        done: Done,
    ) {
        if !self.current_act(generation, enabled) {
            done(Ok(()));
            return;
        }
        let (method, params) = match step {
            0 => (PICKER, json!({"enabled": enabled})),
            1 => (FOCUS, json!({"enabled": enabled})),
            2 => {
                let mut params = auto_attach();
                // A frame born during an act cannot run before its interception is acknowledged.
                params["waitForDebuggerOnStart"] = json!(enabled);
                ("Target.setAutoAttach", params)
            }
            3 if !session.is_empty() => ("Runtime.runIfWaitingForDebugger", json!({})),
            _ => {
                done(Ok(()));
                return;
            }
        };
        let (frames, owner) = (self.clone(), session.to_owned());
        self.transport.call(
            session,
            method,
            params,
            Box::new(move |result| {
                if !frames.current_act(generation, enabled) {
                    done(Ok(()));
                } else if result.is_ok() {
                    frames.control_session(&owner, generation, enabled, step + 1, false, done);
                } else if enabled {
                    if frames.state.borrow().act.done.is_none() {
                        // An already admitted act may still be running. A new frame whose controls fail must
                        // not be resumed with that act's activation and unconfirmed picker interception.
                        frames.close_act_view(Box::new(move |result| {
                            if let Err(error) = result {
                                tracing::error!("browser control teardown: {error}");
                            }
                            done(Err(REFUSED.into()));
                        }));
                    } else {
                        done(Err(REFUSED.into()));
                    }
                } else if !retried {
                    frames.control_session(&owner, generation, false, step, true, done);
                } else {
                    frames.tear_down_session(owner, generation, done);
                }
            }),
        );
    }

    fn tear_down_session(&self, session: String, generation: u64, done: Done) {
        if session.is_empty() {
            self.close_act_view(done);
            return;
        }
        let frames = self.clone();
        self.transport.call(
            "",
            "Target.detachFromTarget",
            json!({"sessionId": session}),
            Box::new(move |result| {
                if !frames.current_act(generation, false) {
                    done(Ok(()));
                } else if result.is_ok() {
                    let mut state = frames.state.borrow_mut();
                    state.sessions.remove(&session);
                    state.generation += 1;
                    drop(state);
                    done(Ok(()));
                } else {
                    frames.close_act_view(done);
                }
            }),
        );
    }

    pub(super) fn close_act_view(&self, done: Done) {
        {
            let mut state = self.state.borrow_mut();
            state.act.closed = true;
            if state.act.enabled {
                state.act.failure.get_or_insert_with(|| REFUSED.into());
            }
        }
        done(self.transport.close());
    }

    /// Sends a control to every current session and collects every completion, including errors.
    pub(super) fn call_every_session(&self, method: &str, params: &Value, done: Done) {
        let sessions: Vec<_> = std::iter::once(String::new())
            .chain(self.state.borrow().sessions.iter().cloned())
            .collect();
        let pending = Rc::new(RefCell::new((sessions.len(), None::<String>, Some(done))));
        for session in sessions {
            let pending = pending.clone();
            self.transport.call(
                &session,
                method,
                params.clone(),
                Box::new(move |result| {
                    let completion = {
                        let mut pending = pending.borrow_mut();
                        if let Err(error) = result {
                            pending.1.get_or_insert(error);
                        }
                        pending.0 -= 1;
                        (pending.0 == 0).then(|| (pending.2.take(), pending.1.take()))
                    };
                    if let Some((Some(done), error)) = completion {
                        done(error.map_or(Ok(()), Err));
                    }
                }),
            );
        }
    }
}
