//! Frame evaluation in an isolated script world over the Chromium `DevTools` protocol, for engines whose only
//! isolated-world API is that protocol (`WebView2`). Every frame of the page, including frames that run in another
//! process, gets a world of its own that page scripts cannot reach; results come from the protocol's answer,
//! never through anything the page can override. The registry generation is host-owned: it changes on every
//! frame lifecycle event the protocol reports and whenever the frame tree read before an evaluation differs
//! from the one read before the previous evaluation.
//!
//! Depends on: `serde_json` and a [`CdpTransport`] (the engine adapter's protocol channel). Called by the engine
//! adapter on its UI thread; single-threaded (`Rc`), answers may arrive synchronously or later.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use serde_json::{Value, json};

use crate::FrameAnswer;

#[path = "cdp_act.rs"]
mod act;
use act::Act;

/// Completes one protocol call with its result object, or the protocol's error text.
pub type CdpDone = Box<dyn FnOnce(Result<Value, String>)>;
/// Completes one frame evaluation.
pub type FramesDone = Box<dyn FnOnce(Result<Vec<FrameAnswer>, String>)>;

/// The engine's `DevTools` protocol channel.
pub trait CdpTransport {
    /// Sends `method` to `session` (`""`: the page's own session). Calls `done` exactly once, now or later.
    fn call(&self, session: &str, method: &str, params: Value, done: CdpDone);

    /// Closes the view if a failed control cannot be cleared by detaching its child session.
    /// A transport without a native view fails closed until the caller disposes of it.
    ///
    /// # Errors
    /// The native view could not be closed, or this transport does not own one.
    fn close(&self) -> Result<(), String> {
        Err("the browser view must be closed".into())
    }
}

/// Protocol events the engine adapter forwards to [`CdpFrames::on_event`].
pub const CDP_EVENTS: [&str; 6] = [
    "Page.frameAttached",
    "Page.frameDetached",
    "Page.frameNavigated",
    "Page.documentOpened",
    "Target.attachedToTarget",
    "Target.detachedFromTarget",
];

/// The private world's name. Page scripts cannot address a world by name.
const WORLD: &str = "vibe-work-agent";
/// Frames evaluated per call (the other engines keep the same bound).
const MAX_FRAMES: usize = 64;
/// Child sessions for frames in other processes.
const MAX_SESSIONS: usize = 64;
/// Never return page exception text: it may contain a filled value.
const FAILED: &str = "frame document changed, detached, or could not be evaluated";

/// The failure with the step that failed and, for a protocol refusal (never a page exception), its short text.
fn failed(step: &str, protocol: Option<&str>) -> String {
    match protocol {
        Some(error) => format!(
            "{FAILED} ({step}: {})",
            error.chars().take(120).collect::<String>()
        ),
        None => format!("{FAILED} ({step})"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {
    id: String,
    session: String,
    loader: String,
    url: String,
    main: bool,
}

#[derive(Default)]
struct State {
    generation: u64,
    act: Act,
    /// Child sessions of frames in other processes, attached automatically.
    sessions: BTreeSet<String>,
    /// The frame tree read by the previous evaluation: id -> (session, document loader).
    seen: BTreeMap<String, (String, String)>,
}

/// Isolated-world frames of one page. Cheap to clone; all clones share one registry.
#[derive(Clone)]
pub struct CdpFrames {
    transport: Rc<dyn CdpTransport>,
    state: Rc<RefCell<State>>,
    /// This evaluation runs as the person's gesture (an agent act's own input).
    gesture: bool,
    await_promise: bool,
}

fn auto_attach() -> Value {
    // Frames in other processes become sessions of their own; nothing waits for the debugger.
    json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true,
        "filter": [{"type": "iframe", "exclude": false}]})
}

impl CdpFrames {
    /// A registry over `transport`; call [`Self::install`] before the page's first navigation.
    #[must_use]
    pub fn new(transport: Rc<dyn CdpTransport>) -> Self {
        Self {
            transport,
            state: Rc::default(),
            gesture: false,
            await_promise: false,
        }
    }

    /// The current host-owned registry generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.state.borrow().generation
    }

    /// Enables frame lifecycle events and automatic attachment to frames in other processes.
    pub fn install(&self, done: Box<dyn FnOnce(Result<(), String>)>) {
        let transport = self.transport.clone();
        self.transport.call(
            "",
            "Page.enable",
            json!({}),
            Box::new(move |result| match result {
                Err(error) => done(Err(error)),
                Ok(_) => transport.call(
                    "",
                    "Target.setAutoAttach",
                    auto_attach(),
                    Box::new(move |result| done(result.map(drop))),
                ),
            }),
        );
    }

    /// Feeds one protocol event (from any session). Lifecycle events change the generation.
    pub fn on_event(&self, session: &str, method: &str, params: &Value) {
        let _ = session;
        let attach = {
            let mut state = self.state.borrow_mut();
            match method {
                "Target.attachedToTarget" => {
                    let child = params["sessionId"].as_str().unwrap_or_default();
                    state.generation += 1;
                    if !child.is_empty()
                        && !state.sessions.contains(child)
                        && state.sessions.len() >= MAX_SESSIONS
                        && (state.act.enabled
                            || state.act.running
                            || params["waitingForDebugger"] == true)
                    {
                        // A paused target outside the bounded registry cannot join control/cleanup rounds.
                        drop(state);
                        self.close_act_view(Box::new(|result| {
                            if let Err(error) = result {
                                tracing::error!("browser session overflow teardown: {error}");
                            }
                        }));
                        return;
                    }
                    if params["targetInfo"]["type"] == "iframe"
                        && !child.is_empty()
                        && state.sessions.len() < MAX_SESSIONS
                        && state.sessions.insert(child.to_owned())
                    {
                        Some(child.to_owned())
                    } else {
                        None
                    }
                }
                "Target.detachedFromTarget" => {
                    if let Some(child) = params["sessionId"].as_str() {
                        state.sessions.remove(child);
                    }
                    state.generation += 1;
                    None
                }
                _ if CDP_EVENTS.contains(&method) => {
                    state.generation += 1;
                    None
                }
                _ => None,
            }
        };
        // The child's own events and its nested frames in further processes; answers are not needed.
        if let Some(child) = attach {
            self.transport
                .call(&child, "Page.enable", json!({}), Box::new(drop));
            if !self.state.borrow().act.enabled && !self.state.borrow().act.running {
                self.transport.call(
                    &child,
                    "Target.setAutoAttach",
                    auto_attach(),
                    Box::new(drop),
                );
            }
            self.sync_act();
        }
    }

    /// Evaluates `script` in the isolated world of every frame (`None`) or of one frame id, with no fallback to
    /// another frame. Each answer carries the native URL (cleared when the evaluated document's own address does
    /// not match the frame tree), the registry generation at completion and the JSON-encoded result.
    pub fn evaluate(&self, target: Option<&str>, script: &str, done: FramesDone) {
        self.evaluate_once(target, script, done, true);
    }

    /// [`Self::evaluate`] for an agent act's own input: the script runs as a user gesture (`userGesture`), so a
    /// window it opens is not blocked, as for a person's click.
    pub fn evaluate_input(&self, target: Option<&str>, script: &str, done: FramesDone) {
        if let Err(error) = self.act_ready() {
            done(Err(error));
            return;
        }
        let mut frames = self.clone();
        frames.gesture = true;
        frames.evaluate_once(target, script, done, true);
    }

    /// Turns the engine's file picker interception on or off in every session: while on, a picker the page opens
    /// never shows; the engine reports it (`Page.fileChooserOpened`). On only while an agent act runs.
    pub fn intercept_file_choosers(&self, enabled: bool, done: act::Done) {
        self.call_every_session(
            "Page.setInterceptFileChooserDialog",
            &json!({"enabled": enabled}),
            done,
        );
    }

    /// Turns focus emulation on or off in every session: while on, the page counts as focused, though the person's
    /// keyboard focus stays where it is (the view never takes it). An agent act's click then gets what a page
    /// allows a click in a focused document, as for a person's click: a Copy button's clipboard write. On only while
    /// an agent act runs.
    pub fn emulate_focus(&self, enabled: bool, done: act::Done) {
        self.call_every_session(
            "Emulation.setFocusEmulationEnabled",
            &json!({"enabled": enabled}),
            done,
        );
    }

    fn evaluate_once(&self, target: Option<&str>, script: &str, done: FramesDone, retry: bool) {
        let sessions: Vec<String> = std::iter::once(String::new())
            .chain(self.state.borrow().sessions.iter().cloned())
            .collect();
        let trees = Rc::new(RefCell::new(Trees {
            left: sessions.len(),
            frames: Vec::new(),
            error: None,
            dead: Vec::new(),
        }));
        let next = Rc::new(RefCell::new(Some(Next {
            frames: self.clone(),
            target: target.map(str::to_owned),
            script: script.to_owned(),
            done,
        })));
        for session in sessions {
            let (trees, next) = (trees.clone(), next.clone());
            let owner = session.clone();
            self.transport.call(
                &session,
                "Page.getFrameTree",
                json!({}),
                Box::new(move |result| {
                    let finished = {
                        let mut trees = trees.borrow_mut();
                        match result {
                            Ok(tree) => collect(&tree["frameTree"], &owner, &mut trees.frames, 0),
                            Err(error) if !owner.is_empty() => trees.dead.push((owner, error)),
                            Err(error) => {
                                trees
                                    .error
                                    .get_or_insert(failed("frame list", Some(&error)));
                            }
                        }
                        trees.left -= 1;
                        (trees.left == 0).then(|| {
                            (
                                std::mem::take(&mut trees.frames),
                                trees.error.take(),
                                std::mem::take(&mut trees.dead),
                            )
                        })
                    };
                    let Some((frames, error, dead)) = finished else {
                        return;
                    };
                    let Some(next) = next.borrow_mut().take() else {
                        return;
                    };
                    if let Some(error) = error {
                        (next.done)(Err(error));
                    } else if let Some((_, error)) = dead.first() {
                        next.recover(&dead, error, retry);
                    } else {
                        next.run(frames);
                    }
                }),
            );
        }
    }
}

#[cfg(test)]
#[path = "cdp_act_tests.rs"]
mod act_tests;

struct Trees {
    left: usize,
    frames: Vec<Frame>,
    error: Option<String>,
    /// Child sessions that could not list their frames (closed without a detach event reaching us).
    dead: Vec<(String, String)>,
}

struct Next {
    frames: CdpFrames,
    target: Option<String>,
    script: String,
    done: FramesDone,
}

/// Flattens one session's frame tree. A frame in another process appears only in its own session's tree.
fn collect(tree: &Value, session: &str, frames: &mut Vec<Frame>, depth: usize) {
    let frame = &tree["frame"];
    let Some(id) = frame["id"].as_str() else {
        return;
    };
    if depth > 16 || frames.len() > MAX_SESSIONS * MAX_FRAMES {
        return;
    }
    let url = format!(
        "{}{}",
        frame["url"].as_str().unwrap_or_default(),
        frame["urlFragment"].as_str().unwrap_or_default()
    );
    frames.push(Frame {
        id: id.to_owned(),
        session: session.to_owned(),
        loader: frame["loaderId"].as_str().unwrap_or_default().to_owned(),
        url,
        main: session.is_empty() && frame["parentId"].is_null(),
    });
    for child in tree["childFrames"].as_array().into_iter().flatten() {
        collect(child, session, frames, depth + 1);
    }
}

fn without_fragment(url: &str) -> &str {
    url.split_once('#').map_or(url, |(base, _)| base)
}

impl Next {
    /// A child session that cannot list its frames is gone for us: forget it and move the generation (a checked
    /// click across this fails closed), ask again for every frame in another process (a live one returns under a
    /// new session before that answer), then read once more.
    fn recover(self, dead: &[(String, String)], error: &str, retry: bool) {
        {
            let mut state = self.frames.state.borrow_mut();
            for (session, _) in dead {
                state.sessions.remove(session);
            }
            state.generation += 1;
        }
        if !retry {
            (self.done)(Err(failed("frame list", Some(error))));
            return;
        }
        let Self {
            frames,
            target,
            script,
            done,
        } = self;
        let again = frames.clone();
        frames.recover_attachment(Box::new(move |result| match result {
            Ok(()) => again.evaluate_once(target.as_deref(), &script, done, false),
            Err(error) => done(Err(error)),
        }));
    }

    fn run(self, mut frames: Vec<Frame>) {
        // A frame listed by two sessions belongs to the session where it runs (not the main session).
        let mut owned = BTreeMap::new();
        for frame in frames.iter().filter(|f| !f.session.is_empty()) {
            owned
                .entry(frame.id.clone())
                .or_insert(frame.session.clone());
        }
        frames.retain(|f| owned.get(&f.id).is_none_or(|session| *session == f.session));
        let mut ids = BTreeSet::new();
        if !frames.iter().all(|f| ids.insert(f.id.clone()))
            || frames.iter().filter(|f| f.main).count() != 1
        {
            (self.done)(Err("frame identity is ambiguous".into()));
            return;
        }
        // Main first, then tree order; the comparison covers every frame, including those over the bound.
        frames.sort_by_key(|f| !f.main);
        {
            let mut state = self.frames.state.borrow_mut();
            let seen: BTreeMap<_, _> = frames
                .iter()
                .map(|f| (f.id.clone(), (f.session.clone(), f.loader.clone())))
                .collect();
            if seen != state.seen {
                state.generation += 1;
                state.seen = seen;
            }
        }
        frames.truncate(MAX_FRAMES);
        let selected: Vec<Frame> = frames
            .into_iter()
            .filter(|f| self.target.as_deref().is_none_or(|t| t == f.id))
            .collect();
        if selected.is_empty() {
            (self.done)(Err("frame is stale or detached".into()));
            return;
        }
        let pending = Rc::new(RefCell::new(Pending {
            left: selected.len(),
            answers: vec![None; selected.len()],
            error: None,
            done: Some(self.done),
            registry: self.frames.clone(),
        }));
        // In an evaluation of every frame, a child that cannot be evaluated answers as unreadable (the browser
        // leaves it out of a read; a checked click refuses while it is there); the main frame's failure fails all.
        let every = self.target.is_none();
        for (index, frame) in selected.into_iter().enumerate() {
            let tolerated = every && !frame.main;
            evaluate_frame(
                &self.frames,
                frame,
                &self.script,
                index,
                &pending,
                tolerated,
            );
        }
    }
}

struct Pending {
    left: usize,
    answers: Vec<Option<(Frame, String)>>,
    error: Option<String>,
    done: Option<FramesDone>,
    registry: CdpFrames,
}

fn finish(pending: &Rc<RefCell<Pending>>, index: usize, answer: Result<(Frame, String), String>) {
    let complete = {
        let mut state = pending.borrow_mut();
        match answer {
            Ok(answer) => state.answers[index] = Some(answer),
            Err(error) => {
                state.error.get_or_insert(error);
            }
        }
        state.left -= 1;
        if state.left == 0 {
            state.done.take().map(|done| {
                let generation = state.registry.generation();
                let result = match state.error.take() {
                    Some(error) => Err(error),
                    None => Ok(std::mem::take(&mut state.answers)
                        .into_iter()
                        .flatten()
                        .map(|(frame, value)| FrameAnswer {
                            id: frame.id,
                            generation: Some(generation),
                            main: frame.main,
                            url: frame.url,
                            origin: None,
                            value,
                        })
                        .collect()),
                };
                (done, result)
            })
        } else {
            None
        }
    };
    if let Some((done, result)) = complete {
        done(result);
    }
}

/// The answer object of `Runtime.evaluate`, by value. An exception never yields its page text.
fn by_value(answer: &Value) -> Result<Value, String> {
    if !answer["exceptionDetails"].is_null() || answer["result"].is_null() {
        return Err(failed("script", None));
    }
    Ok(answer["result"]
        .get("value")
        .cloned()
        .unwrap_or(Value::Null))
}

/// World, document address, then the script: all three in the same execution context, which belongs to exactly
/// one document of the frame (a replaced document has a new context; the old id no longer evaluates).
fn evaluate_frame(
    frames: &CdpFrames,
    mut frame: Frame,
    script: &str,
    index: usize,
    pending: &Rc<RefCell<Pending>>,
    tolerated: bool,
) {
    let transport = frames.transport.clone();
    let act_gate = frames.clone();
    let gesture = frames.gesture;
    let await_promise = frames.await_promise;
    let (pending, script) = (pending.clone(), script.to_owned());
    let listed = frame.clone();
    // A tolerated child's failure is an unreadable answer, never its error text.
    let finish = move |pending: &Rc<RefCell<Pending>>,
                       index: usize,
                       answer: Result<(Frame, String), String>| {
        let answer = match answer {
            Err(_) if tolerated => Ok((listed.clone(), crate::UNREADABLE_FRAME.to_owned())),
            other => other,
        };
        finish(pending, index, answer);
    };
    let finish = Rc::new(finish);
    let session = frame.session.clone();
    frames.transport.call(
        &frame.session.clone(),
        "Page.createIsolatedWorld",
        json!({"frameId": frame.id, "worldName": WORLD, "grantUniveralAccess": false}),
        Box::new(move |world| {
            let finish = finish.clone();
            let context = match world {
                Ok(world) => world["executionContextId"].as_i64(),
                Err(error) => {
                    finish(&pending, index, Err(failed("script world", Some(&error))));
                    return;
                }
            };
            let Some(context) = context else {
                finish(&pending, index, Err(failed("script world", None)));
                return;
            };
            let follow = transport.clone();
            let finish_href = finish.clone();
            transport.call(
                &session.clone(),
                "Runtime.evaluate",
                json!({"expression": "location.href", "contextId": context, "returnByValue": true, "silent": true}),
                Box::new(move |href| {
                    let finish = finish_href;
                    let href = match href {
                        Ok(answer) => by_value(&answer),
                        Err(error) => Err(failed("document address", Some(&error))),
                    };
                    let Ok(Value::String(href)) = href else {
                        let error = href.err().unwrap_or_else(|| failed("document address", None));
                        finish(&pending, index, Err(error));
                        return;
                    };
                    // The tree was read before this context was found: a document that replaced it meanwhile
                    // has no confirmed native address.
                    frame.url = if without_fragment(&href) == without_fragment(&frame.url) {
                        href
                    } else {
                        String::new()
                    };
                    if gesture && let Err(error) = act_gate.act_ready() {
                        finish(&pending, index, Err(error));
                        return;
                    }
                    follow.call(
                        &session,
                        "Runtime.evaluate",
                        json!({"expression": script, "contextId": context, "returnByValue": true, "silent": true,
                            "userGesture": gesture, "awaitPromise": await_promise}),
                        Box::new(move |answer| {
                            let answer = match answer {
                                Ok(answer) => by_value(&answer),
                                Err(error) => Err(failed("script", Some(&error))),
                            }
                            .map(|value| (frame, value.to_string()));
                            finish(&pending, index, answer);
                        }),
                    );
                }),
            );
        }),
    );
}

#[cfg(test)]
#[path = "cdp_frames_tests.rs"]
mod tests;
